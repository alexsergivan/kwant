//! 33e notice-says-stopped-or-blocked: when the person is away, the notification is the only
//! signal, so a turn that stopped or ended Blocked says so instead of "replied" (or nothing).
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
    db::{parked::NewParkedTurn, spaces::SpaceId, threads::NewDelegation, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{continuity, create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// `CRASH` dies with no output (a model that stopped without an answer); `BLOCKED`, `KEYED` (a
/// Blocked line naming a key) and `DONE` end with that line; anything else is a plain answer.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *CRASH*) exit 3 ;;
  *BLOCKED*) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"I looked for the backup.\n\nBlocked: the backup drive isn'\''t mounted. Plug it in and tell me to try again."}' ;;
  *KEYED*) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"GitHub said no.\n\nBlocked: GitHub refused ghp_abcdefghijklmnop1234 so paste a new token in Settings."}' ;;
  *DONE*) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"Here is the plan.\n\nDone: 3-day plan saved, with the museum on day 2."}' ;;
  *) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"answer"}' ;;
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

/// Researcher and Writer on the fake claude; the notifier runs (as `serve` starts it).
async fn setup() -> Env {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    // One turn per message here: no keep-going turn after a reply
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
    // A fresh install is seeded with @helpers (Assistant, Researcher, Writer): they go (Assistant
    // last, as a space keeps one bot), so the two made here are the only bots, on the fake claude
    let seeded = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    for k in seeded.iter().filter(|k| k.name != "Assistant") {
        ok(&app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
    }
    for name in ["Researcher", "Writer"] {
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

    /// The person writes `prompt` to `name` in its direct chat.
    async fn say_to(&self, name: &str, prompt: &str) {
        let id = self.kwant_id(name).await;
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"kwant_id": id, "prompt": prompt, "sender_id": "user"}))).await;
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

    /// Every notification before a plain completed turn of `marker_kwant` that is sent now: the
    /// notifier handles events in order, so whatever came before has been handled by then.
    async fn notes_before_marker(&mut self, marker_kwant: &str) -> Vec<Note> {
        let _ = self.state.event_sender.send(turn_ended(marker_kwant, None, "completed", None, None));
        let mut notes = self.notes_until("turn_finished").await;
        let marker = notes.pop().unwrap();
        assert!(marker.title.ends_with(" replied"), "{marker:?}");
        notes
    }
}

fn turn_ended(kwant_id: &str, thread_id: Option<&str>, outcome: &str, reply: Option<&str>, context: Option<&str>) -> RuntimeEvent {
    RuntimeEvent::TurnEnded {
        kwant_id: kwant_id.into(),
        context_id: Some(context.or(thread_id).unwrap_or(kwant_id).into()),
        outcome: outcome.into(),
        turn_id: None,
        thread_id: thread_id.map(Into::into),
        quiet: false,
        reply: reply.map(Into::into),
    }
}

#[tokio::test]
async fn a_failed_turn_says_the_bot_stopped() {
    let mut env = setup().await;
    env.say_to("Researcher", "CRASH please find the flights").await;
    let notes = env.notes_until("turn_stopped").await;
    assert_eq!(
        notes.last().unwrap(),
        &Note {
            kind: "turn_stopped".into(),
            title: "Researcher stopped".into(),
            body: "Researcher couldn't finish in your direct chat. Open kwant to retry.".into(),
        }
    );
    // One notification for the stop, and no "replied"
    let rest = env.notes_before_marker(&env.kwant_id("Writer").await).await;
    assert!(rest.is_empty(), "{rest:?}");
    assert!(notes.iter().all(|n| n.kind != "turn_finished"), "{notes:?}");
}

/// A turn parked behind a usage limit retries by itself at the reset, and its chat says when:
/// "stopped… Open kwant to retry" would ask the person for nothing (Rigel, 33e QA).
#[tokio::test]
async fn a_turn_waiting_for_its_limit_retry_sends_no_stop() {
    let mut env = setup().await;
    let kid = env.kwant_id("Researcher").await;
    let parked = env
        .state
        .db
        .park_turn(&NewParkedTurn {
            kwant_id: kid.clone(),
            channel_id: None,
            direct_kwant_id: Some(kid.clone()),
            thread_id: None,
            delegation_id: None,
            depth: 0,
            prompt: "find the flights".into(),
            retry_at: chrono::Utc::now() + chrono::Duration::hours(1),
            reason: "usage_limit",
        })
        .await
        .unwrap();
    assert!(parked.is_some());
    env.say_to("Researcher", "CRASH please find the flights").await;
    // Its turn ends failed; every notification up to then is kept
    let mut notes = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match env.rx.recv().await.map(|e| e.unscoped().clone()) {
                Ok(RuntimeEvent::TurnEnded { kwant_id, outcome, .. }) if kwant_id == kid => {
                    assert_eq!(outcome, "failed");
                    break;
                }
                Ok(RuntimeEvent::Notify { kind, title, body, .. }) => notes.push(Note { kind, title, body }),
                _ => {}
            }
        }
    })
    .await
    .expect("the turn ends");
    notes.extend(env.notes_before_marker(&env.kwant_id("Writer").await).await);
    assert!(notes.iter().all(|n| n.kind != "turn_stopped"), "{notes:?}");
}

#[tokio::test]
async fn a_blocked_reply_says_the_bot_is_blocked() {
    let mut env = setup().await;
    env.say_to("Writer", "BLOCKED back up the drafts").await;
    let notes = env.notes_until("turn_stopped").await;
    assert_eq!(
        notes.last().unwrap(),
        &Note {
            kind: "turn_stopped".into(),
            title: "Writer is blocked".into(),
            body: "Blocked: the backup drive isn't mounted. Plug it in and tell me to try again.".into(),
        }
    );
    let rest = env.notes_before_marker(&env.kwant_id("Researcher").await).await;
    assert!(rest.is_empty(), "{rest:?}");
    assert!(notes.iter().all(|n| n.kind != "turn_finished" && !n.title.contains("replied")), "{notes:?}");
}

/// The Blocked line leaves kwant (push, webhook, Telegram): a key in it is masked.
#[tokio::test]
async fn a_key_in_a_blocked_line_is_masked() {
    let mut env = setup().await;
    env.say_to("Writer", "KEYED open the pull request").await;
    let note = env.notes_until("turn_stopped").await.pop().unwrap();
    assert_eq!(note.title, "Writer is blocked");
    assert!(!note.body.contains("ghp_"), "{note:?}");
    // Masked by `missions::autofix::redact`, which writes `[redacted]` (impl.md, privacy note)
    assert_eq!(note.body, "Blocked: GitHub refused [redacted] so paste a new token in Settings.");
}

#[tokio::test]
async fn a_done_reply_says_what_was_done() {
    let mut env = setup().await;
    env.say_to("Researcher", "DONE plan my trip").await;
    let notes = env.notes_until("turn_finished").await;
    assert_eq!(
        notes,
        vec![Note {
            kind: "turn_finished".into(),
            title: "Researcher replied".into(),
            body: "Done: 3-day plan saved, with the museum on day 2.".into(),
        }]
    );
}

#[tokio::test]
async fn a_plain_reply_still_says_replied() {
    let mut env = setup().await;
    env.say_to("Researcher", "hello there").await;
    let notes = env.notes_until("turn_finished").await;
    assert_eq!(
        notes,
        vec![Note {
            kind: "turn_finished".into(),
            title: "Researcher replied".into(),
            body: "Researcher finished replying in your direct chat.".into(),
        }]
    );
}

/// A delegated member reports through its lead, a cancelled turn was the person's own Stop, and a
/// quiet turn's runner notifies for it: none of them sends a notification of its own.
#[tokio::test]
async fn delegated_cancelled_and_quiet_turns_send_nothing_of_their_own() {
    let mut env = setup().await;
    let (researcher, writer) = (env.kwant_id("Researcher").await, env.kwant_id("Writer").await);
    let (thread, _) = env
        .state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: researcher.clone(),
            specialist_kwant_id: writer.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(researcher.clone()),
            origin_thread_id: None,
            team_id: None,
            task: "Write the intro".into(),
            title: "Intro".into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    let blocked = "Blocked: the backup drive isn't mounted. Plug it in and tell me to try again.";
    let send = |e: RuntimeEvent| {
        let _ = env.state.event_sender.send(e);
    };
    send(turn_ended(&writer, Some(&thread.id), "failed", None, None));
    send(turn_ended(&writer, Some(&thread.id), "completed", Some(blocked), None));
    send(turn_ended(&writer, None, "cancelled", None, None));
    send(turn_ended(&writer, None, "cancelled", Some(blocked), None));
    let mut quiet = turn_ended(&writer, None, "failed", None, None);
    if let RuntimeEvent::TurnEnded { quiet: q, .. } = &mut quiet {
        *q = true;
    }
    send(quiet);
    let notes = env.notes_before_marker(&researcher).await;
    assert!(notes.is_empty(), "{notes:?}");
}

/// A routine run that fails already sends `routine_failed`; its turn adds no second notice.
#[tokio::test]
async fn a_failed_routine_run_sends_only_routine_failed() {
    let mut env = setup().await;
    let kid = env.kwant_id("Researcher").await;
    let r = ok(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": kid, "name": "Digest", "instructions": "CRASH summarise yesterday.",
            "schedule": {"type": "daily", "time": "09:00", "tz": "Europe/Kyiv"}})),
    )
    .await;
    let id = r["id"].as_str().unwrap().to_string();
    let (s, _) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let notes = env.notes_until("routine_failed").await;
    let rest = env.notes_before_marker(&env.kwant_id("Writer").await).await;
    let all: Vec<&Note> = notes.iter().chain(rest.iter()).collect();
    assert!(all.iter().all(|n| n.kind != "turn_stopped"), "{all:?}");
}
