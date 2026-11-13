//! The model badge's server side: `turn_runs_on` says which engine and model a turn runs on as
//! the driver starts (again on failover), and `last_ran_on` on GET /api/kwants keeps what each
//! bot's newest reply ran on across restarts. Fake claude and codex CLIs on PATH.
//! Its own test binary: it changes PATH, HOME and the process env.

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
    db::Database,
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;
use tower::ServiceExt;

/// Prints Claude's session-limit line as its whole reply while `$FAKE_DIR/claude.out` exists.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "7.0.1 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0 ;;
esac
echo "$*" | tr "\n" " " >> "$FAKE_DIR/claude.argv"; echo >> "$FAKE_DIR/claude.argv"
if [ -f "$FAKE_DIR/claude.out" ]; then
  echo '{"type":"result","is_error":false,"num_turns":1,"result":"You'"'"'ve hit your session limit · resets 3am (Europe/Berlin)"}'
  exit 0
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"hello from fake claude","usage":{"input_tokens":3,"output_tokens":4}}'
"#;

const FAKE_CODEX: &str = r#"#!/bin/sh
case "$1 $2" in
  "--version "*) echo "codex-cli 0.162.1"; exit 0 ;;
  "login status") echo "Logged in using ChatGPT"; exit 0 ;;
  "debug models") cat <<'JSON'
{"models":[
 {"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","description":"Latest workhorse model","visibility":"list","priority":1},
 {"slug":"gpt-6-astra","display_name":"GPT-6 Astra","description":"Frontier intelligence for the most demanding work","visibility":"list","priority":2},
 {"slug":"gpt-6-luna","display_name":"GPT-6 Luna","description":"Fast and affordable model for easier tasks","visibility":"list","priority":3}]}
JSON
  exit 0 ;;
  "mcp list") echo '[]'; exit 0 ;;
esac
echo "$*" | tr "\n" " " >> "$FAKE_DIR/codex.argv"; echo >> "$FAKE_DIR/codex.argv"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-o" ]; then out="$a"; fi
  prev="$a"
done
[ -n "$out" ] && echo "hello from fake codex" > "$out"
exit 0
"#;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn app_on(data: &Path) -> (Router, AppState) {
    let db = Database::init(data).await.unwrap();
    let config = Config {
        data_dir: data.to_path_buf(),
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
    (create_router(state.clone()), state)
}

/// Sends a prompt and collects the turn's events (scoped as sent) up to `turn_ended`.
async fn run_turn(app: &Router, state: &AppState, kwant_id: &str) -> Vec<RuntimeEvent> {
    let mut rx = state.event_sender.subscribe();
    let (s, v) = call(app, "POST", "/api/chat/send", Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            let end = matches!(e.unscoped(), RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id);
            events.push(e);
            if end {
                break;
            }
        }
    })
    .await
    .expect("turn did not end");
    events
}

/// The turn's `turn_runs_on` events, as the wire sends them.
fn runs_on(events: &[RuntimeEvent]) -> Vec<Value> {
    events.iter().filter(|e| matches!(e.unscoped(), RuntimeEvent::TurnRunsOn { .. })).map(|e| e.to_wire()).collect()
}

/// Where each kind of event sits in the turn: the first index of each.
fn first(events: &[RuntimeEvent], f: impl Fn(&RuntimeEvent) -> bool) -> usize {
    events.iter().position(|e| f(e.unscoped())).unwrap_or_else(|| panic!("missing: {events:?}"))
}

async fn last_reply_meta(state: &AppState, kwant_id: &str) -> Value {
    let msgs = state.db.list_messages(kwant_id, 50).await.unwrap();
    let m = msgs.iter().rev().find(|m| m.sender_role == "kwant").expect("a saved reply");
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

async fn kwant(app: &Router, id: &str) -> Value {
    let (s, list) = call(app, "GET", "/api/kwants", None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    list.as_array().unwrap().iter().find(|k| k["id"] == id).cloned().unwrap_or_else(|| panic!("{id} in {list}"))
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

/// The event's model fields match the saved reply's `ran_on`.
fn same_model(ev: &Value, ran_on: &Value) {
    for k in ["engine", "family", "model", "model_label", "tier"] {
        assert_eq!(ev["data"][k], ran_on[k], "{k}: {ev} vs {ran_on}");
    }
}

async fn pin(app: &Router, id: &str, model: &str) {
    let mut body = kwant(app, id).await;
    body.as_object_mut().unwrap().retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!(model);
    let (s, v) = call(app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn turns_say_what_they_run_on_and_the_bot_list_remembers_it() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for (name, script) in [("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX)] {
        exe::write_executable(bin.join(name), script);
    }
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("HOME", tmp.path());
    std::env::set_var("FAKE_DIR", tmp.path());
    for var in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "XAI_API_KEY", "OLLAMA_MODEL", "CLAUDE_CONFIG_DIR"] {
        std::env::remove_var(var);
    }
    std::env::set_var("OLLAMA_URL", "http://127.0.0.1:9");

    let data = tmp.path().join("data");
    let (app, state) = app_on(&data).await;
    let (s, v) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for (id, priority) in [("prf_system_claude", 10), ("prf_system_codex", 20)] {
        let (s, v) = call(&app, "PATCH", &format!("/api/profiles/{id}"), Some(json!({"priority": priority}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    // 1. A new bot has never run: last_ran_on is there, and null (so is a starter bot's)
    let (s, created) = call(&app, "POST", "/api/kwants", Some(json!({"name": "Badge", "title": "Badge tester", "persona_prompt": "Test persona",
        "avatar_sprite": "s", "driver_type": "api", "preferred_model": "auto"}))).await;
    assert!(s.is_success(), "{s} {created}");
    let new_id = created["id"].as_str().unwrap().to_string();
    let fresh = kwant(&app, &new_id).await;
    assert!(fresh.as_object().unwrap().contains_key("last_ran_on") && fresh["last_ran_on"].is_null(), "{fresh}");
    let (_, list) = call(&app, "GET", "/api/kwants", None).await;
    let kid = list[0]["id"].as_str().unwrap().to_string();
    assert_ne!(kid, new_id);
    let before = kwant(&app, &kid).await;
    assert!(before["last_ran_on"].is_null(), "{before}");
    pin(&app, &kid, "auto").await;

    // 2. Auto: one turn_runs_on with Laya's pick (Laya offline: "hi" is the fast tier, Haiku),
    // after turn_started and before the reply's first text, scoped like turn_started
    let events = run_turn(&app, &state, &kid).await;
    let ro = runs_on(&events);
    assert_eq!(ro.len(), 1, "{events:?}");
    let ev = &ro[0];
    assert_eq!(ev["type"], "turn_runs_on");
    assert_eq!(
        keys(&ev["data"]),
        ["context_id", "engine", "family", "kwant_id", "model", "model_label", "picked_by", "tier", "turn_id"].map(String::from).into(),
        "{ev}"
    );
    let d = &ev["data"];
    assert_eq!(
        (d["kwant_id"].clone(), d["engine"].clone(), d["family"].clone(), d["model"].clone(), d["tier"].clone(), d["picked_by"].clone()),
        (json!(kid), json!("claude"), json!("claude"), json!("haiku"), json!("fast"), json!("laya")),
        "{ev}"
    );
    assert!(d["model_label"].is_string() && d["turn_id"].is_string() && d["context_id"].is_string(), "{ev}");
    let started = first(&events, |e| matches!(e, RuntimeEvent::TurnStarted { .. }));
    let runs = first(&events, |e| matches!(e, RuntimeEvent::TurnRunsOn { .. }));
    let text = first(&events, |e| matches!(e, RuntimeEvent::TextDelta(_)));
    assert!(started < runs && runs < text, "{started} {runs} {text}: {events:?}");
    let RuntimeEvent::TurnStarted { context_id, turn_id, .. } = events[started].unscoped() else { unreachable!() };
    assert_eq!((Some(d["context_id"].as_str().unwrap()), Some(d["turn_id"].as_str().unwrap())), (context_id.as_deref(), turn_id.as_deref()));

    // An older client's payload is unchanged: turn_started's shape, the reply's ran_on keys
    let wire = events[started].to_wire();
    assert_eq!(wire["type"], "turn_started");
    assert_eq!(keys(&wire["data"]), ["context_id", "kwant_id", "turn_id"].map(String::from).into(), "{wire}");
    let meta = last_reply_meta(&state, &kid).await;
    let ran_on = meta["ran_on"].clone();
    assert_eq!(
        keys(&ran_on),
        ["account", "effort", "engine", "engine_name", "family", "model", "model_label", "picked_by", "profile_id", "tier"].map(String::from).into(),
        "{ran_on}"
    );
    assert_eq!(ran_on["picked_by"], "laya", "the reply keeps its own vocabulary");
    same_model(ev, &ran_on);

    // 3. last_ran_on right after the reply: the reply's model, Laya's pick, its time
    let after = kwant(&app, &kid).await;
    let last = after["last_ran_on"].clone();
    for k in ["engine", "family", "model", "model_label", "tier"] {
        assert_eq!(last[k], ran_on[k], "{k}: {last} vs {ran_on}");
    }
    assert_eq!(last["picked_by"], "laya");
    let at = chrono::DateTime::parse_from_rfc3339(last["at"].as_str().unwrap()).unwrap();
    assert!((chrono::Utc::now() - at.with_timezone(&chrono::Utc)).num_seconds().abs() < 60, "{last}");
    // Every key the list had before is still there; only last_ran_on is new
    assert!(keys(&before).is_subset(&keys(&after)), "{before} vs {after}");

    // 4. A pinned model: picked_by "pinned" on both
    pin(&app, &kid, "claude/sonnet").await;
    let events = run_turn(&app, &state, &kid).await;
    let ro = runs_on(&events);
    assert_eq!(ro.len(), 1, "{events:?}");
    assert_eq!((ro[0]["data"]["model"].clone(), ro[0]["data"]["picked_by"].clone()), (json!("sonnet"), json!("pinned")), "{ro:?}");
    let ran_on = last_reply_meta(&state, &kid).await["ran_on"].clone();
    assert_eq!(ran_on["picked_by"], "pinned_model");
    same_model(&ro[0], &ran_on);
    let last = kwant(&app, &kid).await["last_ran_on"].clone();
    assert_eq!((last["model"].clone(), last["picked_by"].clone()), (json!("sonnet"), json!("pinned")), "{last}");

    // 5. Failover: Claude is out, so a second turn_runs_on names Codex, picked_by "fallback"
    pin(&app, &kid, "claude").await;
    std::fs::write(tmp.path().join("claude.out"), "").unwrap();
    let events = run_turn(&app, &state, &kid).await;
    let ro = runs_on(&events);
    assert_eq!(ro.len(), 2, "{events:?}");
    assert_eq!((ro[0]["data"]["engine"].clone(), ro[0]["data"]["picked_by"].clone()), (json!("claude"), json!("pinned")), "{ro:?}");
    assert_eq!((ro[1]["data"]["engine"].clone(), ro[1]["data"]["picked_by"].clone()), (json!("codex"), json!("fallback")), "{ro:?}");
    assert_eq!(ro[0]["data"]["turn_id"], ro[1]["data"]["turn_id"], "one turn");
    let sw = first(&events, |e| matches!(e, RuntimeEvent::ProfileSwitched { .. }));
    let second = events.iter().rposition(|e| matches!(e.unscoped(), RuntimeEvent::TurnRunsOn { .. })).unwrap();
    assert!(sw < second, "the switch line comes first: {events:?}");
    let ran_on = last_reply_meta(&state, &kid).await["ran_on"].clone();
    assert_eq!(ran_on["engine"], "codex");
    same_model(&ro[1], &ran_on);
    let last = kwant(&app, &kid).await["last_ran_on"].clone();
    assert_eq!((last["engine"].clone(), last["picked_by"].clone()), (json!("codex"), json!("fallback")), "{last}");
    // A later turn that skips Claude unasked also says so from the start
    let ro = runs_on(&run_turn(&app, &state, &kid).await);
    assert_eq!(ro.len(), 1, "{ro:?}");
    assert_eq!((ro[0]["data"]["engine"].clone(), ro[0]["data"]["picked_by"].clone()), (json!("codex"), json!("fallback")), "{ro:?}");
    let last = kwant(&app, &kid).await["last_ran_on"].clone();

    // 6. A restart: the same last_ran_on for every bot, read back from the database
    let all_before: Vec<(Value, Value)> = {
        let (_, list) = call(&app, "GET", "/api/kwants", None).await;
        list.as_array().unwrap().iter().map(|k| (k["id"].clone(), k["last_ran_on"].clone())).collect()
    };
    drop(app);
    let (app2, _state2) = app_on(&data).await;
    assert_eq!(kwant(&app2, &kid).await["last_ran_on"], last);
    let (_, list) = call(&app2, "GET", "/api/kwants", None).await;
    let all_after: Vec<(Value, Value)> = list.as_array().unwrap().iter().map(|k| (k["id"].clone(), k["last_ran_on"].clone())).collect();
    assert_eq!(all_after, all_before);
    assert_eq!(kwant(&app2, &new_id).await["last_ran_on"], Value::Null, "a bot that never ran stays null");
}
