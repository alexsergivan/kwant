//! Laya picks the model tier as well as the engine, pins choose engine or model, and engines
//! turned off in Settings are never checked, routed to or offered to Laya. Fake claude, codex
//! and grok CLIs log their argv. Its own test binary: it changes PATH, HOME and the process env.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::{detect_available_models, engines::EngineId, engines::EngineStatus, RuntimeEvent},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "7.0.1 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0 ;;
esac
echo "$*" | tr "\n" " " >> "$FAKE_DIR/claude.argv"; echo >> "$FAKE_DIR/claude.argv"
echo '{"type":"result","is_error":false,"num_turns":1,"result":"hello from fake claude","usage":{"input_tokens":3,"output_tokens":4}}'
"#;

/// Shaped like the real `codex debug models` on this host.
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

const FAKE_GROK: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "grok 1.0.62 (1474f9cbdd91)"; exit 0 ;;
  models) printf 'Default model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4-fast\n  - grok-4-heavy\n'; exit 0 ;;
esac
echo "$*" | tr "\n" " " >> "$FAKE_DIR/grok.argv"; echo >> "$FAKE_DIR/grok.argv"
echo '{"type":"text","data":"hello from fake grok"}'
echo '{"type":"end","stopReason":"end_turn"}'
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

/// Sends `prompt` and returns (streamed text, thinking notes).
async fn turn(app: &Router, state: &AppState, kwant_id: &str, prompt: &str) -> (String, Vec<String>) {
    let mut rx = state.event_sender.subscribe();
    let (s, v) = call(
        app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kwant_id, "prompt": prompt, "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (mut text, mut notes) = (String::new(), Vec::new());
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            match e.unscoped() {
                RuntimeEvent::TextDelta(t) => text.push_str(t),
                RuntimeEvent::ThinkingDelta(t) => notes.push(t.clone()),
                RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id => break,
                _ => {}
            }
        }
    })
    .await
    .expect("turn did not end");
    (text, notes)
}

/// The last argv a fake CLI logged.
fn last_argv(dir: &Path, cli: &str) -> String {
    std::fs::read_to_string(dir.join(format!("{cli}.argv")))
        .unwrap_or_default()
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

fn runs(dir: &Path, cli: &str) -> usize {
    std::fs::read_to_string(dir.join(format!("{cli}.argv"))).map(|s| s.lines().count()).unwrap_or(0)
}

/// `meta.ran_on` of the kwant's newest saved reply.
async fn ran_on(state: &AppState, kwant_id: &str) -> Value {
    let msgs = state.db.list_messages(kwant_id, 50).await.unwrap();
    msgs.iter()
        .filter(|m| m.sender_role == "kwant")
        .filter_map(|m| m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()))
        .filter_map(|m| m.get("ran_on").cloned())
        .next_back()
        .expect("a reply with meta.ran_on")
}

fn by_id(v: &Value, id: &str) -> Value {
    v["engines"].as_array().unwrap().iter().find(|e| e["id"] == id).cloned().unwrap()
}

#[tokio::test]
async fn laya_picks_model_tier_pins_and_engine_switches() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for (name, script) in [("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX), ("grok", FAKE_GROK)] {
        let fake = bin.join(name);
        std::fs::write(&fake, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("HOME", tmp.path());
    std::env::set_var("FAKE_DIR", tmp.path());
    for var in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "XAI_API_KEY", "OLLAMA_MODEL", "CLAUDE_CONFIG_DIR"] {
        std::env::remove_var(var);
    }
    std::env::set_var("OLLAMA_URL", "http://127.0.0.1:9");

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
    let app = create_router(state.clone());

    // Every engine starts on, and every model says its tier
    let (s, v) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["engines"].as_array().unwrap().iter().all(|e| e["enabled"] == true), "{v}");
    let tiers = |id: &str| -> Vec<(String, Value)> {
        by_id(&v, id)["models"].as_array().unwrap().iter().map(|m| (m["id"].as_str().unwrap().to_string(), m["tier"].clone())).collect()
    };
    assert_eq!(by_id(&v, "claude")["status"], "ready");
    // Ready on its own login, so a kwant uses This machine; keyless APIs are neither
    assert_eq!(by_id(&v, "claude")["ready"], true);
    assert_eq!(by_id(&v, "claude")["uses"], json!({"profile_id": "prf_system_claude", "label": "This machine"}));
    assert_eq!((by_id(&v, "anthropic_api")["ready"].clone(), by_id(&v, "anthropic_api")["uses"].clone()), (json!(false), Value::Null));
    assert!(tiers("claude").contains(&("haiku".into(), json!("fast"))));
    assert!(tiers("claude").contains(&("opus".into(), json!("deep"))));
    assert_eq!(tiers("codex"), [("gpt-6.1-sol".into(), json!("balanced")), ("gpt-6-astra".into(), json!("deep")), ("gpt-6-luna".into(), json!("fast"))]);
    assert_eq!(tiers("grok"), [("grok-4.6".into(), Value::Null), ("grok-4-fast".into(), json!("fast")), ("grok-4-heavy".into(), json!("deep"))]);

    // 1. Switches: turn codex and grok off. Bad ids are refused; PUT works like POST.
    let (s, _) = call(&app, "POST", "/api/engines/nope", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = call(&app, "POST", "/api/engines/codex", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["engines"].as_array().unwrap().len(), 7, "the full GET /api/engines shape");
    let codex = by_id(&v, "codex");
    assert_eq!((codex["status"].clone(), codex["enabled"].clone(), codex["models"].clone()), (json!("disabled"), json!(false), json!([])));
    assert_eq!((codex["ready"].clone(), codex["uses"].clone()), (json!(false), Value::Null));
    let (_, listed) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(by_id(&listed, "claude")["ready"], true, "GET carries ready/uses too");
    let (s, _) = call(&app, "PUT", "/api/engines/grok", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.db.get_setting("engines_disabled").await.unwrap().as_deref(), Some(r#"["codex","grok"]"#));
    // A re-check leaves them off and unchecked
    let (_, v) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(by_id(&v, "grok")["status"], "disabled");
    assert_eq!(by_id(&v, "claude")["status"], "ready");

    // Laya is offered only engines that are on
    let offered: Vec<String> =
        detect_available_models(&state.db, &state.engines, &HashSet::new(), &HashSet::new()).await.into_iter().map(|m| m.id).collect();
    assert!(offered.contains(&"claude".to_string()), "{offered:?}");
    assert!(!offered.iter().any(|m| m == "codex" || m == "grok"), "{offered:?}");

    // 2. Auto: a trivial prompt runs Haiku, a hard one Opus (Laya offline: keyword effort)
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut()
        .unwrap()
        .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    let pin = |body: &mut Value, p: &str| body["preferred_model"] = json!(p);
    pin(&mut body, "auto");
    let (s, r) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK, "{r}");

    let (text, notes) = turn(&app, &state, &kid, "hi").await;
    assert!(text.contains("hello from fake claude"), "{text}");
    assert!(last_argv(tmp.path(), "claude").contains("--model haiku"), "{}", last_argv(tmp.path(), "claude"));
    assert!(notes.iter().any(|n| n.contains("Running on Claude Code CLI · This machine · haiku (fast tier)")), "{notes:?}");
    let r = ran_on(&state, &kid).await;
    assert_eq!(
        (r["engine"].clone(), r["family"].clone(), r["model"].clone(), r["tier"].clone(), r["picked_by"].clone(), r["effort"].clone()),
        (json!("claude"), json!("claude"), json!("haiku"), json!("fast"), json!("laya"), json!("low")),
        "{r}"
    );
    assert!(r["model_label"].is_string() && r["engine_name"].is_string(), "{r}");

    turn(&app, &state, &kid, "Investigate the root cause of this deadlock and propose an architecture refactor").await;
    assert!(last_argv(tmp.path(), "claude").contains("--model opus"), "{}", last_argv(tmp.path(), "claude"));
    let r = ran_on(&state, &kid).await;
    assert_eq!((r["model"].clone(), r["tier"].clone()), (json!("opus"), json!("deep")), "{r}");

    // 3. Pin forms: model fixed whatever the effort; old model-id pins still work
    for (p, want) in [("claude/sonnet", "--model sonnet"), ("claude:haiku", "--model haiku"), ("claude-opus-5-5", "--model opus")] {
        pin(&mut body, p);
        call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
        turn(&app, &state, &kid, "Investigate the root cause of this deadlock").await;
        assert!(last_argv(tmp.path(), "claude").contains(want), "{p}: {}", last_argv(tmp.path(), "claude"));
        assert_eq!(ran_on(&state, &kid).await["picked_by"], "pinned_model", "{p}");
    }
    // An engine pin: Laya's effort picks the tier on it
    pin(&mut body, "claude");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    turn(&app, &state, &kid, "hi").await;
    assert!(last_argv(tmp.path(), "claude").contains("--model haiku"));
    assert_eq!(ran_on(&state, &kid).await["picked_by"], "pinned_engine");

    // 4. A pin to an engine that is off never runs it
    pin(&mut body, "codex");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    let (text, _) = turn(&app, &state, &kid, "hi").await;
    assert!(text.contains("can't run right now"), "{text}");
    assert_eq!(runs(tmp.path(), "codex"), 0);

    // 5. Back on: probed at once, and -m reaches codex and grok
    let (s, v) = call(&app, "POST", "/api/engines/codex", Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((by_id(&v, "codex")["status"].clone(), by_id(&v, "codex")["enabled"].clone()), (json!("ready"), json!(true)));
    call(&app, "POST", "/api/engines/grok", Some(json!({"enabled": true}))).await;
    assert_eq!(state.engines.status(EngineId::Grok), EngineStatus::Ready);
    assert_eq!(state.db.get_setting("engines_disabled").await.unwrap().as_deref(), Some("[]"));

    turn(&app, &state, &kid, "hi").await;
    assert!(last_argv(tmp.path(), "codex").starts_with("exec -m gpt-6-luna "), "{}", last_argv(tmp.path(), "codex"));
    turn(&app, &state, &kid, "Investigate the root cause of this deadlock").await;
    assert!(last_argv(tmp.path(), "codex").starts_with("exec -m gpt-6-astra "), "{}", last_argv(tmp.path(), "codex"));
    pin(&mut body, "codex/gpt-6.1-sol");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    turn(&app, &state, &kid, "hi").await;
    assert!(last_argv(tmp.path(), "codex").starts_with("exec -m gpt-6.1-sol "), "{}", last_argv(tmp.path(), "codex"));

    pin(&mut body, "grok");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    let (text, _) = turn(&app, &state, &kid, "Investigate the root cause of this deadlock").await;
    assert!(text.contains("hello from fake grok"), "{text}");
    assert!(last_argv(tmp.path(), "grok").contains("-m grok-4-heavy"), "{}", last_argv(tmp.path(), "grok"));
    let r = ran_on(&state, &kid).await;
    assert_eq!((r["engine"].clone(), r["model"].clone(), r["tier"].clone()), (json!("grok"), json!("grok-4-heavy"), json!("deep")), "{r}");
    pin(&mut body, "grok:grok-4.6");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    turn(&app, &state, &kid, "hi").await;
    assert!(last_argv(tmp.path(), "grok").contains("-m grok-4.6"), "{}", last_argv(tmp.path(), "grok"));
}
