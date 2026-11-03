//! A bot whose engine is out of usage is answered by the next account in the one Accounts order,
//! across engines, with fake claude and codex CLIs on PATH. The limit line itself is never a reply.
//! Its own test binary: it changes PATH, HOME and the process env.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{Duration as Span, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::{engines::EngineId, RuntimeEvent},
    profiles::classify::{FailClass, ProviderFailure},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
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

/// Sends a prompt and collects the turn's events up to `turn_ended`.
async fn run_turn(app: &Router, state: &AppState, kwant_id: &str) -> Vec<RuntimeEvent> {
    let mut rx = state.event_sender.subscribe();
    let (s, v) = call(app, "POST", "/api/chat/send", Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            let end = matches!(&e, RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id);
            events.push(e.unscoped().clone());
            if end {
                break;
            }
        }
    })
    .await
    .expect("turn did not end");
    events
}

fn text_of(events: &[RuntimeEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn switches(events: &[RuntimeEvent]) -> Vec<&RuntimeEvent> {
    events.iter().filter(|e| matches!(e, RuntimeEvent::ProfileSwitched { .. })).collect()
}

fn runs(dir: &Path, cli: &str) -> usize {
    std::fs::read_to_string(dir.join(format!("{cli}.argv"))).map(|s| s.lines().count()).unwrap_or(0)
}

/// The newest saved reply of the bot: its text and parsed meta.
async fn last_reply(state: &AppState, kwant_id: &str) -> (String, Value) {
    let msgs = state.db.list_messages(kwant_id, 50).await.unwrap();
    let m = msgs.iter().rev().find(|m| m.sender_role == "kwant").expect("a saved reply");
    (m.text.clone(), m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null))
}

/// "03:00 CET" / "04:00 CEST": `at` on a Europe/Ljubljana wall clock, with the zone's abbreviation.
/// Written from the EU rule (summer time runs 01:00 UTC on the last Sunday of March to the same
/// moment in October), not from the code under test.
fn ljubljana(at: chrono::DateTime<Utc>) -> String {
    use chrono::{Datelike, NaiveDate, TimeZone, Weekday};
    let last_sunday = |month: u32| {
        let mut d = NaiveDate::from_ymd_opt(at.year(), month, 31).unwrap();
        while d.weekday() != Weekday::Sun {
            d = d.pred_opt().unwrap();
        }
        Utc.from_utc_datetime(&d.and_hms_opt(1, 0, 0).unwrap())
    };
    let summer = at >= last_sunday(3) && at < last_sunday(10);
    let (hours, name) = if summer { (2, "CEST") } else { (1, "CET") };
    format!("{} {name}", (at + Span::hours(hours)).format("%H:%M"))
}

#[tokio::test]
async fn a_session_limit_moves_the_bot_to_the_next_engine_and_back() {
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
    // Claude is out from the start
    std::fs::write(tmp.path().join("claude.out"), "").unwrap();

    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config {
        data_dir: data.clone(),
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
    // The person's saved zone: server text reads in it and names it
    state.db.set_setting("timezone", "Europe/Ljubljana").await.unwrap();
    let app = create_router(state.clone());
    let (s, v) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // The Accounts order: Claude first, then Codex
    for (id, priority) in [("prf_system_claude", 10), ("prf_system_codex", 20)] {
        let (s, v) = call(&app, "PATCH", &format!("/api/profiles/{id}"), Some(json!({"priority": priority}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // 1. Claude prints the session-limit line: Codex answers, with one quiet notice
    let t0 = Utc::now();
    let events = run_turn(&app, &state, &kid).await;
    let text = text_of(&events);
    assert!(text.contains("hello from fake codex"), "{text}");
    assert!(!text.contains("session limit"), "the limit line is not a reply: {text}");
    assert_eq!(runs(tmp.path(), "claude"), 1);
    assert_eq!(runs(tmp.path(), "codex"), 1);
    let sw = switches(&events);
    assert_eq!(sw.len(), 1, "{events:?}");
    let RuntimeEvent::ProfileSwitched { from_profile_id, to_profile_id, class, retry_at, from_engine, to_engine, to_model, .. } = sw[0] else {
        unreachable!()
    };
    assert_eq!((from_profile_id.as_str(), to_profile_id.as_str(), class.as_str()), ("prf_system_claude", "prf_system_codex", "quota"));
    assert_eq!((from_engine.as_str(), to_engine.as_str()), ("claude", "codex"));
    assert!(to_model.as_deref().is_some_and(|m| m.to_lowercase().contains("gpt-6")), "{to_model:?}");
    assert!(matches!(events.last(), Some(RuntimeEvent::TurnEnded { outcome, .. }) if outcome == "completed"), "{events:?}");

    // The reply is Codex's, carries the notice in its meta, and no limit line is saved anywhere
    let (reply, meta) = last_reply(&state, &kid).await;
    assert_eq!(reply.trim(), "hello from fake codex");
    assert_eq!(meta["ran_on"]["engine"], "codex", "{meta}");
    assert_eq!(meta["failover"].as_array().map(Vec::len), Some(1), "{meta}");
    let ps = &meta["failover"][0];
    assert_eq!((ps["from_engine"].clone(), ps["to_engine"].clone()), (json!("claude"), json!("codex")), "{meta}");
    assert!(ps["to_model"].is_string() && ps["from_profile_id"] == "prf_system_claude" && ps["retry_at"].is_string(), "{meta}");
    // retry_at is RFC 3339 with an offset (so the client localises it) and the notice text names
    // its zone, never the server's or the person's own clock unlabelled
    let stamp = ps["retry_at"].as_str().unwrap();
    let parsed = chrono::DateTime::parse_from_rfc3339(stamp).unwrap_or_else(|e| panic!("retry_at {stamp:?}: {e}"));
    assert!(stamp.ends_with("+00:00") && stamp.contains('T'), "{stamp}");
    assert!(ps["notice"].as_str().unwrap().contains(&format!("is out until {}", ljubljana(parsed.with_timezone(&Utc)))), "{ps}");
    let saved = state.db.list_messages(&kid, 50).await.unwrap();
    assert!(!saved.iter().any(|m| m.sender_role == "kwant" && m.text.contains("session limit")), "{saved:?}");

    // Claude is limited until the next 03:00 in Berlin (01:00 or 02:00 UTC), within a day
    let (_, list) = call(&app, "GET", "/api/profiles", None).await;
    let claude = list.as_array().unwrap().iter().find(|p| p["id"] == "prf_system_claude").unwrap().clone();
    assert_eq!(claude["last_error"], "quota");
    let until: chrono::DateTime<Utc> = claude["limited_until"].as_str().unwrap().parse().unwrap();
    assert_eq!(retry_at.unwrap(), until);
    assert!(until > t0 && until <= t0 + Span::hours(24) + Span::minutes(1), "{until}");
    let (h, m) = (chrono::Timelike::hour(&until), chrono::Timelike::minute(&until));
    assert!(m == 0 && (h == 1 || h == 2), "03:00 Berlin is 01:00 or 02:00 UTC, got {until}");

    // 2. The next turn skips Claude without an attempt, and says nothing more about it
    let events = run_turn(&app, &state, &kid).await;
    assert!(text_of(&events).contains("hello from fake codex"));
    assert_eq!(runs(tmp.path(), "claude"), 1, "no attempt on a known-limited account");
    assert_eq!(runs(tmp.path(), "codex"), 2);
    assert!(switches(&events).is_empty(), "one line per outage: {events:?}");
    assert!(last_reply(&state, &kid).await.1.get("failover").is_none());

    // 3. After the reset the bot goes back to Claude by itself
    std::fs::remove_file(tmp.path().join("claude.out")).unwrap();
    let reset = Utc::now() - Span::minutes(1);
    state.db.record_profile_failure("prf_system_claude", "quota", Some(reset)).await.unwrap();
    // The engine's own "quota exceeded until" mark lapses at the same moment
    state.engines.record_failure(
        EngineId::Claude,
        &ProviderFailure { class: FailClass::Quota, retry_at: Some(reset), detail: String::new() },
    );
    let events = run_turn(&app, &state, &kid).await;
    assert!(text_of(&events).contains("hello from fake claude"), "{events:?}");
    assert_eq!((runs(tmp.path(), "claude"), runs(tmp.path(), "codex")), (2, 2));
    assert!(switches(&events).is_empty());

    // 4. Everything out: nothing is tried, one line says when, and the turn parks once
    state.db.record_profile_failure("prf_system_claude", "quota", Some(Utc::now() + Span::hours(3))).await.unwrap();
    state.db.record_profile_failure("prf_system_codex", "quota", Some(Utc::now() + Span::hours(2))).await.unwrap();
    // The default host-login setup: a real refusal also marks the whole engine "quota exceeded
    // until", which must read as a limit (account still there, earliest reset wins), not as an
    // engine that cannot run
    for (engine, hours) in [(EngineId::Claude, 3), (EngineId::Codex, 2)] {
        state.engines.record_failure(
            engine,
            &ProviderFailure { class: FailClass::Quota, retry_at: Some(Utc::now() + Span::hours(hours)), detail: String::new() },
        );
    }
    let events = run_turn(&app, &state, &kid).await;
    let text = text_of(&events);
    assert!(text.contains("Every model is out until") && text.contains("I'll answer then."), "{text}");
    // The person's zone, named, and the earliest reset (Codex's, in two hours) as written in it
    let wait = Utc::now() + Span::hours(2);
    let near = [wait, wait - Span::minutes(1), wait + Span::minutes(1)].map(|t| format!("Every model is out until {}", ljubljana(t)));
    assert!(near.iter().any(|n| text.contains(n)), "{text}");
    assert_eq!((runs(tmp.path(), "claude"), runs(tmp.path(), "codex")), (2, 2), "nothing was attempted");
    let parked = state.db.list_parked().await.unwrap();
    assert_eq!(parked.len(), 1, "{parked:?}");
    // The earliest reset is Codex's, two hours away
    let at = parked[0].retry_at;
    assert!(at > Utc::now() + Span::minutes(119) && at < Utc::now() + Span::minutes(121), "{at}");
    // A second message while everything is out is parked with the same line, never a setup card
    let events = run_turn(&app, &state, &kid).await;
    let text = text_of(&events);
    assert!(text.contains("Every model is out until") && (text.contains(" CET") || text.contains(" CEST")) && !text.contains("No account can run bots"), "{text}");
    let (reply, meta) = last_reply(&state, &kid).await;
    assert!(!reply.contains("Settings") && meta.get("kind") != Some(&json!("setup_needed")), "{reply} {meta}");
    assert_eq!((runs(tmp.path(), "claude"), runs(tmp.path(), "codex")), (2, 2), "nothing was attempted");
    assert_eq!(state.db.list_parked().await.unwrap().len(), 1, "a chat parks at most once");
    let saved = state.db.list_messages(&kid, 50).await.unwrap();
    assert!(!saved.iter().any(|m| m.text.contains("session limit")));
}
