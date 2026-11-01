//! Hand-off cards don't pile up in Review (Oleks, 2026-10-29: "many tasks in review column, who
//! is going to check them??"). A finished hand-off's result is evidence on its card, the lead's
//! results turn names the card and how to review it, and only the lead's review counts: a pass
//! verifies the card, a fail sends the work back to the teammate with the note, on the same card.
//! Every card in Review gets a lead review turn, or isn't put there: a lead that doesn't review is
//! reminded once, a card the batch turn missed (blocked at the time, or a restart) gets its own
//! turn, and a one-way hand-off is closed. A fake claude CLI on PATH answers every turn with
//! "done" (so a lead never calls task_review). Its own test binary: it changes PATH.

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
        exe::write_executable(&fake, FAKE_CLAUDE);
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

/// Prompts that asked the lead to review card `task_id` (results turn, review turn or reminder).
fn review_asks_for(task_id: &str) -> usize {
    prompts_with(&[&format!("task_id \"{task_id}\"")]).len()
}

async fn asks_logged(env: &Env, task_id: &str) -> usize {
    missions::listener::review_asks(&env.state, task_id).await.0
}

/// Lets background turns that would wrongly start show up before a test counts prompts.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

/// The results turn asks the lead to review; a lead that doesn't gets exactly one reminder,
/// then the card waits, named for its reviewer, and a sweep doesn't ask again.
#[tokio::test]
async fn a_lead_that_doesnt_review_is_reminded_once_and_then_the_card_waits() {
    let env = setup().await;
    let (did, task) = finished(&env, "Tidy the logs REMINDMARK", "Log tidy").await;
    wait_for("the reminder", || async { !prompts_with(&["Reminder:", "Log tidy", &task.id]).is_empty() }).await;
    wait_for("both asks logged", || async { asks_logged(&env, &task.id).await == 2 }).await;
    settle().await;
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle().await;
    assert_eq!(review_asks_for(&task.id), 2, "the results turn and one reminder, nothing more");
    assert_eq!(asks_logged(&env, &task.id).await, 2);
    assert_eq!(card_state(&env, &did).await, "review");
    let t = card(&env, &did).await.unwrap();
    let lead_name = env.state.db.get_kwant(&env.lead).await.unwrap().unwrap().name;
    assert_eq!(missions::task_json(&env.state, &t).await["reviewer"]["name"], json!(lead_name));
}

/// A hand-off that doesn't report back to its lead is a one-way notice: nobody is resumed with
/// its result, so its card is closed with a note instead of waiting in Review.
#[tokio::test]
async fn a_one_way_hand_off_is_closed_instead_of_left_in_review() {
    let env = setup().await;
    let body = json!({"lead_kwant_id": env.lead, "specialist_kwant_id": env.specialist, "task": "FYI the deploy moved NOTICEMARK",
        "title": "Deploy notice", "kwant_id": env.lead, "continue_lead": false});
    let (s, d) = call(&env.app, "POST", "/api/delegations", Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    let did = id(&d);
    wait_for("the card to close", || async { card_state(&env, &did).await == "cancelled" }).await;
    let t = card(&env, &did).await.unwrap();
    assert!(!moves(&env, &t.id).await.iter().any(|m| m.ends_with(">review")), "it never went to review");
    let events = env.state.db.task_events(&t.id, 200).await.unwrap();
    assert!(events.iter().any(|e| e.detail.contains("Notice, no review needed")), "the close says why");
    assert_eq!(missions::task_json(&env.state, &t).await["closed_note"], json!("Notice, no review needed"), "it reads as Closed");
    settle().await;
    assert_eq!(review_asks_for(&t.id), 0, "nobody is asked to review a notice");
}

/// The hand-off finished while its card was blocked, so the lead's results turn didn't see the
/// card. When the person unblocks it, it goes to Review and the lead gets a review turn for it.
#[tokio::test]
async fn a_card_unblocked_after_its_hand_off_finished_gets_a_review_turn() {
    let env = setup().await;
    let release = tempfile::tempdir().unwrap().keep().join("go");
    let d = delegate(&env, &format!("Long job BLOCKMARK LONGTASK:{} ", release.display()), "Long job").await;
    let did = id(&d);
    wait_for("the card to be worked on", || async { card_state(&env, &did).await == "working" }).await;
    let t = card(&env, &did).await.unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/tasks/{}/block", t.id), Some(json!({"reason": "wait for the release"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    std::fs::write(&release, "go").unwrap();
    wait_for("the hand-off to finish", || async { status_of(&env, &did).await == "completed" }).await;
    wait_for("the lead's results turn", || async { !prompts_with(&["finished the task you delegated", "Long job"]).is_empty() }).await;
    settle().await;
    assert_eq!(card_state(&env, &did).await, "blocked");
    assert_eq!(review_asks_for(&t.id), 0, "a blocked card isn't up for review yet");

    let (s, v) = call(&env.app, "POST", &format!("/api/tasks/{}/unblock", t.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("the card in review", || async { card_state(&env, &did).await == "review" }).await;
    wait_for("the lead's review turn", || async { !prompts_with(&["its card is waiting for your review", &t.id]).is_empty() }).await;
    wait_for("the one reminder", || async { asks_logged(&env, &t.id).await == 2 }).await;
}

/// kwnt restarted after the card reached Review but before the lead's turn (here: the hand-off
/// ended without kwant reporting it). The sweep gives the lead its review turn, plus the one
/// reminder, and then leaves the card alone.
#[tokio::test]
async fn the_sweep_asks_the_lead_about_a_card_whose_review_turn_was_lost() {
    let env = setup().await;
    let lead = env.state.db.get_kwant(&env.lead).await.unwrap().unwrap();
    let new = kwnt::db::threads::NewDelegation {
        lead_kwant_id: env.lead.clone(),
        specialist_kwant_id: env.specialist.clone(),
        origin_channel_id: None,
        origin_kwant_id: Some(lead.id.clone()),
        origin_thread_id: None,
        team_id: None,
        task: "Rotate the keys SWEEPMARK".into(),
        title: "Key rotation".into(),
        continue_lead: true,
        depth: 1,
        follow_up_at: None,
    };
    let (_, d) = env.state.db.create_delegation(&new).await.unwrap();
    assert!(env.state.db.finish_delegation(&d.id, "completed", None, None).await.unwrap());
    missions::listener::on_delegation(&env.state, &d.id).await;
    assert_eq!(card_state(&env, &d.id).await, "review");
    let t = card(&env, &d.id).await.unwrap();
    assert_eq!(review_asks_for(&t.id), 0, "nobody asked the lead yet");

    missions::listener::sweep_unreviewed(&env.state, Duration::from_secs(3600)).await;
    settle().await;
    assert_eq!(review_asks_for(&t.id), 0, "within the grace time the sweep waits for the batch turn");

    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    wait_for("the lead's review turn", || async { !prompts_with(&["its card is waiting for your review", &t.id]).is_empty() }).await;
    wait_for("the one reminder", || async { asks_logged(&env, &t.id).await == 2 }).await;
    settle().await;
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle().await;
    assert_eq!(review_asks_for(&t.id), 2, "asked once and reminded once, then left for the lead or the person");
    assert_eq!(card_state(&env, &d.id).await, "review");
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

/// A lead that answers the reminder by reviewing: its pass verifies the card, and nothing asks
/// it again. (The fake CLI can't call tools, so the test makes the lead's task_review call once
/// the reminder has gone out, as the lead would in that turn.)
#[tokio::test]
async fn a_lead_that_reviews_after_the_reminder_verifies_the_card() {
    let env = setup().await;
    let (did, task) = finished(&env, "Check the cron entry ONREMINDMARK", "Cron check").await;
    wait_for("the reminder", || async {
        !prompts_with(&["Reminder:", "Cron check", &task.id]).is_empty()
    })
    .await;
    wait_for("both asks logged", || async {
        asks_logged(&env, &task.id).await == 2
    })
    .await;
    let ev = result_evidence(&env, &task.id).await;
    let (out, err) = review(
        &env,
        &env.lead,
        &task.id,
        "pass",
        &ev,
        "The cron entry is right",
    )
    .await;
    assert!(!err, "{out}");
    assert_eq!(card_state(&env, &did).await, "verified");
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle().await;
    assert_eq!(
        review_asks_for(&task.id),
        2,
        "a verified card is never asked about again"
    );
}

/// Two hand-offs of one lead that finish together: one results prompt names both cards, and the
/// one reminder covers both, in a single turn.
#[tokio::test]
async fn a_batch_of_two_cards_gets_one_prompt_and_one_reminder() {
    let env = setup().await;
    let release = tempfile::tempdir().unwrap().keep().join("release");
    let mut dids = Vec::new();
    for (who, title) in [(&env.specialist, "Batch alpha"), (&env.other, "Batch beta")] {
        let body = json!({"lead_kwant_id": env.lead, "specialist_kwant_id": who, "title": title, "kwant_id": env.lead,
            "task": format!("Part of the batch BATCHMARK LONGTASK:{}", release.display())});
        let (s, d) = call(&env.app, "POST", "/api/delegations", Some(body)).await;
        assert_eq!(s, StatusCode::ACCEPTED, "{d}");
        dids.push(id(&d));
    }
    std::fs::write(&release, "go").unwrap();
    let mut ids = Vec::new();
    for did in &dids {
        wait_for("the card to reach review", || async {
            card_state(&env, did).await == "review"
        })
        .await;
        ids.push(card(&env, did).await.unwrap().id);
    }
    let both = [
        format!("task_id \"{}\"", ids[0]),
        format!("task_id \"{}\"", ids[1]),
    ];
    let both: Vec<&str> = both.iter().map(String::as_str).collect();
    wait_for("the reminder", || async {
        !prompts_with(&[&["Reminder:"][..], &both].concat()).is_empty()
    })
    .await;
    for t in &ids {
        wait_for("both asks logged", || async {
            asks_logged(&env, t).await == 2
        })
        .await;
    }
    settle().await;
    assert_eq!(
        prompts_with(&both).len(),
        2,
        "one results prompt and one reminder, each naming both cards"
    );
    assert_eq!(
        prompts_with(&[&["Reminder:"][..], &both].concat()).len(),
        1,
        "one reminder for the batch"
    );
    for t in &ids {
        assert_eq!(review_asks_for(t), 2, "no card was asked about on its own");
    }
}

/// A hand-off made in the database, finished, that no board event has seen yet (no card).
async fn ended_unseen(env: &Env, task: &str, title: &str) -> kwnt::db::threads::Delegation {
    let new = kwnt::db::threads::NewDelegation {
        lead_kwant_id: env.lead.clone(),
        specialist_kwant_id: env.specialist.clone(),
        origin_channel_id: None,
        origin_kwant_id: Some(env.lead.clone()),
        origin_thread_id: None,
        team_id: None,
        task: task.into(),
        title: title.into(),
        continue_lead: true,
        depth: 1,
        follow_up_at: None,
    };
    let (_, d) = env.state.db.create_delegation(&new).await.unwrap();
    assert!(env.state.db.finish_delegation(&d.id, "completed", None, None).await.unwrap());
    env.state.db.get_delegation(&d.id).await.unwrap().unwrap()
}

/// The lead's results turn can run before the board's listener has seen the hand-off (a slow
/// listener; the batch test timed out on CI for this): its card is made then, in review with its
/// result, and the lead is asked about it in that turn instead of only by the sweep minutes later.
#[tokio::test]
async fn the_lead_is_asked_about_a_card_the_board_had_not_seen_yet() {
    let env = setup().await;
    let d = ended_unseen(&env, "Index the archive UNSEENMARK", "Archive index").await;
    assert!(card(&env, &d.id).await.is_none(), "no event made its card");
    let note = missions::listener::lead_review_note(&env.state, &d, "Specialist").await;
    let t = card(&env, &d.id).await.expect("the results turn made the card");
    assert!(note.is_some_and(|n| n.contains(&t.id)), "the lead is asked about it in this turn");
    assert_eq!(t.state, "review");
    assert_eq!(result_evidence(&env, &t.id).await.len(), 1, "with its result");
    assert_eq!(asks_logged(&env, &t.id).await, 1);
}

/// A results turn that runs late, after the lead's review already sent the work back, carries
/// the hand-off as it was when it ended. It must not push the card of the fix still running to
/// review: the fix's own result would then never be attached (CI, #170: one result, not two).
#[tokio::test]
async fn a_late_results_turn_leaves_the_card_of_a_running_fix_alone() {
    let env = setup().await;
    let ended = ended_unseen(&env, "Fix the importer STALEMARK", "Importer fix").await;
    missions::listener::on_delegation(&env.state, &ended.id).await;
    assert_eq!(card_state(&env, &ended.id).await, "review");
    let t = card(&env, &ended.id).await.unwrap();
    let ev = result_evidence(&env, &t.id).await;
    let release = tempfile::tempdir().unwrap().keep().join("fixed");
    let (out, err) = review(&env, &env.lead, &t.id, "fail", &ev, &format!("Handle empty rows LONGTASK:{} ", release.display())).await;
    assert!(!err, "{out}");
    wait_for("the fix to run", || async { status_of(&env, &ended.id).await == "running" && card_state(&env, &ended.id).await == "working" }).await;

    let note = missions::listener::lead_review_note(&env.state, &ended, "Specialist").await;
    assert!(note.is_none(), "the old end isn't up for review any more");
    assert_eq!(card_state(&env, &ended.id).await, "working", "the fix's card stays with the fix");

    std::fs::write(&release, "go").unwrap();
    wait_for("the fixed result in review", || async { card_state(&env, &ended.id).await == "review" }).await;
    assert_eq!(result_evidence(&env, &t.id).await.len(), 2, "the fix's result is attached too");
}
