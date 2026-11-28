//! mention-pushes-say-what (follow-up of pushes-say-what, #1365): a team lead that hands work on
//! with an @mention in its team-channel reply pushes who got the task, not "<lead> replied" (the
//! "done already?" of Mira's 0.9.57 team-job pass, fix #4). Its turn ends before the mention
//! hand-offs exist, so the push waits for them. A mention that hands nothing on still says replied.
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
    db::{spaces::SpaceId, Database},
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

/// The lead hearing its results, then a teammate's hand-off, are checked first (their prompts quote
/// the request). `HANDON` hands Researcher a task, `NOBODY` @mentions a name nobody has, and a
/// "hi team" is greeted with a mention; anything else is a plain answer.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *"finished the task you delegated"*|*"tasks you delegated have ended"*) out="Here are three caterers for the party." ;;
  *"mentioned you"*) out="Three caterers under 30 euros a head." ;;
  *HANDON*) out="On it.\n@Researcher, find three caterers in Ljubljana" ;;
  *NOBODY*) out="On it.\n@Nobody, find three caterers in Ljubljana" ;;
  *"hi team"*) out="Hi! @Researcher is here too" ;;
  *) out="Sure." ;;
esac
printf '%s\n' "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"$out\"}"
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
    db.set_setting("mission_board", "false").await.unwrap();
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

    /// Team `team` led by Lead, with Researcher and Writer; its channel's id.
    async fn team_channel(&self, team: &str) -> String {
        let ids = [self.kwant_id("Lead").await, self.kwant_id("Researcher").await, self.kwant_id("Writer").await];
        ok(&self.app, "POST", "/api/teams", Some(json!({"name": team, "lead_kwant_id": ids[0], "member_kwants": ids}))).await;
        let channels = ok(&self.app, "GET", "/api/channels", None).await;
        channels.as_array().unwrap().iter().find(|c| c["name"] == format!("team-{team}").as_str()).unwrap_or_else(|| panic!("no team-{team}: {channels}"))
            ["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn say_in(&self, channel: &str, prompt: &str) {
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": prompt, "sender_id": "user"}))).await;
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

    /// The lead's first turn in `channel` ends: its notification (the only `turn_finished` before it).
    async fn lead_push(&mut self) -> Note {
        let notes = self.notes_until("turn_finished").await;
        notes.into_iter().last().unwrap()
    }

    async fn hand_offs_from(&self, channel: &str) -> Vec<kwnt::db::threads::Delegation> {
        let lead = self.kwant_id("Lead").await;
        let all = self.state.db.list_delegations(Some(&lead), None, 20).await.unwrap();
        all.into_iter().filter(|d| d.origin_channel_id.as_deref() == Some(channel)).collect()
    }
}

/// "@Researcher, find three caterers" in the lead's team-channel reply hands the work on; the
/// push said "Lead replied · Lead finished replying in #team-party." 1 s after the ask.
#[tokio::test]
async fn a_leads_mention_says_who_got_the_task() {
    let mut env = setup().await;
    let channel = env.team_channel("party").await;
    env.say_in(&channel, "@Lead Plan the office party HANDON").await;
    let push = env.lead_push().await;
    let handed = env.hand_offs_from(&channel).await;
    assert_eq!(handed.len(), 1, "one hand-off from the mention: {handed:?}");
    assert_eq!(handed[0].title, "Find three caterers in Ljubljana");
    assert!(handed[0].lead_turn_id.is_none(), "a mention hand-off keeps no lead turn (it would hold its result)");
    assert_eq!(
        push,
        note("turn_finished", "Lead handed “Find three caterers in Ljubljana” to Researcher", "Results come back to #team-party.")
    );
}

/// A mention that hands nothing on keeps "replied": a greeting that names a teammate, and a name
/// nobody here has.
#[tokio::test]
async fn a_mention_that_hands_nothing_on_still_says_replied() {
    let mut env = setup().await;
    let channel = env.team_channel("hello").await;
    env.say_in(&channel, "@Lead hi team").await;
    assert_eq!(env.lead_push().await, note("turn_finished", "Lead replied", "Lead finished replying in #team-hello."));
    assert!(env.hand_offs_from(&channel).await.is_empty(), "a greeting hands nothing on");

    let mut env = setup().await;
    let channel = env.team_channel("nobody").await;
    env.say_in(&channel, "@Lead Plan the office party NOBODY").await;
    assert_eq!(env.lead_push().await, note("turn_finished", "Lead replied", "Lead finished replying in #team-nobody."));
    assert!(env.hand_offs_from(&channel).await.is_empty(), "nobody is called Nobody");
}

/// A plain lead reply without a mention isn't held back and says replied, as before.
#[tokio::test]
async fn a_lead_reply_without_a_mention_says_replied() {
    let mut env = setup().await;
    let channel = env.team_channel("plain").await;
    env.say_in(&channel, "@Lead Plan the office party").await;
    assert_eq!(env.lead_push().await, note("turn_finished", "Lead replied", "Lead finished replying in #team-plain."));
}
