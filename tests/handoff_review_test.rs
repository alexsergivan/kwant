//! Hand-off cards don't pile up in Review (Oleks, 2026-10-29: "many tasks in review column, who
//! is going to check them??"). A finished hand-off's result is evidence on its card, the lead's
//! results turn names the card and how to review it, and only the lead's review counts: a pass
//! verifies the card, a fail sends the work back to the teammate with the note, on the same card.
//! A fake claude CLI on PATH answers every turn with "done". Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{missions::Task, Database},
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, missions, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$$-$(date +%s%N)"
case "$last" in
esac
release=$(printf '%s\n' "$last" | sed -n 's/.*LONGTASK:\([^ ]*\).*/\1/p' | head -n 1)
if [ -n "$release" ]; then
  i=0
  while [ ! -f "$release" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"done"}'
"#;

fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        prompts
    })
}

/// Prompts the fake CLI got that contain every one of `markers`.
fn prompts_with(markers: &[&str]) -> Vec<String> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| markers.iter().all(|m| p.contains(m)))
        .collect()
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}


struct Env {
    app: Router,
    state: AppState,
    lead: String,
    specialist: String,
    other: String,
}

async fn setup() -> Env {
    prompts_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Laya is unreachable: turns use the kwant's preferred model, the fake claude
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
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(3) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    let (s, v) = call(&app, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The board follows hand-offs on the event bus, as the scheduler starts it in kwnt
    missions::listener::spawn(state.clone(), tokio_util::sync::CancellationToken::new());
    std::mem::forget(tmp);
    Env { app, state, lead: ids[0].clone(), specialist: ids[1].clone(), other: ids[2].clone() }
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A hand-off from the lead to the specialist that reports back to the lead.
async fn delegate(env: &Env, task: &str, title: &str) -> Value {
    let body = json!({"lead_kwant_id": env.lead, "specialist_kwant_id": env.specialist, "task": task, "title": title, "kwant_id": env.lead});
    let (s, d) = call(&env.app, "POST", "/api/delegations", Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    d
}


async fn status_of(env: &Env, id: &str) -> String {
    env.state.db.get_delegation(id).await.unwrap().map(|d| d.status).unwrap_or_default()
}

async fn card(env: &Env, delegation_id: &str) -> Option<Task> {
    env.state.db.task_by_source("delegation", delegation_id).await.unwrap()
}

async fn card_state(env: &Env, delegation_id: &str) -> String {
    card(env, delegation_id).await.map(|t| t.state).unwrap_or_default()
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// Moves of the task, as "from>to".
async fn moves(env: &Env, task_id: &str) -> Vec<String> {
    let events = env.state.db.task_events(task_id, 200).await.unwrap();
    events.iter().filter_map(|e| Some(format!("{}>{}", e.from_state.as_deref()?, e.to_state.as_deref()?))).collect()
}


async fn review(env: &Env, by: &str, task_id: &str, verdict: &str, evidence: &[String], note: &str) -> (Value, bool) {
    let bot = env.state.db.get_kwant(by).await.unwrap().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let args = json!({"task_id": task_id, "verdict": verdict, "evidence_ids": evidence, "note": note});
    let (text, is_error) = missions::tools::call(&env.state, &scope, "task_review", &args).await;
    (serde_json::from_str(&text).unwrap_or(Value::String(text)), is_error)
}

/// A finished hand-off waiting in review, with its result attached: (delegation id, card).
async fn finished(env: &Env, task: &str, title: &str) -> (String, Task) {
    let d = delegate(env, task, title).await;
    let did = id(&d);
    wait_for("the card to reach review", || async { card_state(env, &did).await == "review" }).await;
    (did.clone(), card(env, &did).await.unwrap())
}

async fn result_evidence(env: &Env, task_id: &str) -> Vec<String> {
    let ev = env.state.db.list_evidence(task_id).await.unwrap();
    ev.iter().filter(|e| e.label == missions::listener::RESULT_LABEL).map(|e| e.id.clone()).collect()
}

/// The lead is told which card to review and how; its pass verifies the card.
#[tokio::test]
async fn the_lead_reviews_a_finished_hand_off_and_a_pass_verifies_it() {
    let env = setup().await;
    let (did, task) = finished(&env, "Check the parser PASSMARK", "Parser check").await;
    let ev = result_evidence(&env, &task.id).await;
    assert_eq!(ev.len(), 1, "the result is attached once");
    wait_for("the lead's results turn", || async { !prompts_with(&["finished the task you delegated", "Parser check"]).is_empty() }).await;
    let lead_prompt = prompts_with(&["finished the task you delegated", "Parser check"]).remove(0);
    assert!(lead_prompt.contains(&task.id) && lead_prompt.contains("task_review") && lead_prompt.contains(&ev[0]), "{lead_prompt}");

    let card_json = missions::task_json(&env.state, &task).await;
    assert_eq!(card_json["reviewer"]["kwant_id"], json!(env.lead), "the card names its reviewer");

    let (out, err) = review(&env, &env.lead, &task.id, "pass", &ev, "Parser handles the new format").await;
    assert!(!err, "{out}");
    assert_eq!(card_state(&env, &did).await, "verified");
}

/// A fail goes back to Ready with the lead's note and on to the teammate in the hand-off's own
/// thread; the fixed result comes back to review on the same card, for the lead to review again.
#[tokio::test]
async fn a_failed_review_sends_the_work_back_to_the_teammate_with_the_reason() {
    let env = setup().await;
    let (did, task) = finished(&env, "Write the backend fix FAILMARK", "Backend fix").await;
    let ev = result_evidence(&env, &task.id).await;
    let note = "The retry path has no test NOTEMARK";
    let (out, err) = review(&env, &env.lead, &task.id, "fail", &ev, note).await;
    assert!(!err, "{out}");

    let m = moves(&env, &task.id).await;
    assert!(m.iter().any(|x| x == "review>ready"), "{m:?}");
    let events = env.state.db.task_events(&task.id, 200).await.unwrap();
    let back = events.iter().find(|e| e.from_state.as_deref() == Some("review") && e.to_state.as_deref() == Some("ready")).unwrap();
    assert!(back.detail.contains("peer_review"), "the move gives its reason: {}", back.detail);

    wait_for("the teammate's fix turn", || async { !prompts_with(&["sent it back", "NOTEMARK", "FAILMARK"]).is_empty() }).await;
    wait_for("the fixed result back in review", || async {
        moves(&env, &task.id).await.iter().filter(|x| *x == "working>review").count() == 2 && card_state(&env, &did).await == "review"
    })
    .await;
    assert_eq!(status_of(&env, &did).await, "completed");
    let ev = result_evidence(&env, &task.id).await;
    assert_eq!(ev.len(), 2, "the new result is attached too");
    wait_for("the lead's second results turn", || async { !prompts_with(&["after it was sent back", "Backend fix", "task_review"]).is_empty() }).await;

    let space = env.state.db.space_of_kwant(&env.lead).await.unwrap().unwrap();
    let cards = env.state.db.list_tasks(&space, None, None, Some("delegation"), 200).await.unwrap();
    assert_eq!(cards.iter().filter(|t| t.source_id.as_deref() == Some(did.as_str())).count(), 1, "one hand-off, one card");
}

/// The teammate who did the work can't approve it, and neither can a bystander: only the lead.
#[tokio::test]
async fn only_the_lead_reviews_a_hand_off_card() {
    let env = setup().await;
    let (did, task) = finished(&env, "Draft the release notes OWNERMARK", "Release notes").await;
    let ev = result_evidence(&env, &task.id).await;
    let (out, err) = review(&env, &env.specialist, &task.id, "pass", &ev, "looks good to me").await;
    assert!(err, "the owner's own review is refused: {out}");
    let (out, err) = review(&env, &env.other, &task.id, "pass", &ev, "fine").await;
    assert!(err && out.to_string().contains("who handed this task out"), "{out}");
    assert_eq!(card_state(&env, &did).await, "review");
    assert!(env.state.db.list_checks(&task.id).await.unwrap().is_empty(), "no review was recorded");
}

/// Each hand-off from one message is named after what the lead asked that teammate.
#[test]
fn a_hand_off_is_named_after_what_its_teammate_was_asked() {
    use kwnt::server::routes::handoff_title;
    let text = "On it.\n@QA, run the smoke tests on staging. Then report.\n@Backend: fix the rancher reconnect\n@QA please re-check the login flow!";
    assert_eq!(handoff_title(text, "QA").as_deref(), Some("Run the smoke tests on staging."));
    assert_eq!(handoff_title(text, "backend").as_deref(), Some("Fix the rancher reconnect"));
    assert_eq!(handoff_title("**@Parser** — parse the new feed", "Parser").as_deref(), Some("Parse the new feed"));
    assert_eq!(handoff_title("Thanks @QA.", "QA"), None, "nothing asked: the message's title is kept");
    assert_eq!(handoff_title("no mention here", "QA"), None);
    assert_eq!(handoff_title("@Social @Writer draft the launch post", "Social"), None, "one line for several teammates names none of them apart");
}
