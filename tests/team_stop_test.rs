//! 32h: Stop on a teammate stops the team (Mira's UX pass at 0.9.35, fix #1). Stopping a hand-off
//! that another teammate waits for leaves that teammate Stopped (not Failed, not in Needs you),
//! and nothing restarts the work: no keep-going turn and no "Keep going?" card. Stopping the
//! lead's summary of its hand-offs' results runs no review turn after it, now or from the sweep,
//! also after a server restart (32o): the Stop is kept on the card, not only in memory.
//!
//! A fake claude CLI on PATH answers as the bot named in its system prompt ("You are <name>, …"),
//! from files under `$KWNT_TEST_BEHAVE`: `<name>.<kind>` if it exists, else `<name>` for a turn of
//! kind `other`. Kinds: `res` (a lead resumed with results), `kg` (a keep-going turn), `review`
//! (a lead's review turn), `other` (anything else). A first line "WAIT:<file>" holds the turn
//! until that file exists. Every prompt is saved as `$KWNT_TEST_PROMPTS/<name>.<kind>-<ns>`.
//! Its own test binary: it changes PATH.

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
    db::{models::Message, Database},
    router::LayaRouter,
    server::{create_router, keep_going, missions, AppState},
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
me=$(printf '%s\n' "$@" | sed -n 's/.*You are \([A-Za-z0-9]*\), .*/\1/p' | head -n 1)
kind=other
case "$last" in
  *"[Decide first]"*) kind=decide ;;
  *"Review each one now"*) kind=review ;;
  *"isn't marked done"*) kind=kg ;;
  *"you delegated"*) kind=res ;;
esac
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$me.$kind-$(date +%s%N)"
file="$KWNT_TEST_BEHAVE/$me.$kind"
[ -f "$file" ] || { [ "$kind" = other ] && file="$KWNT_TEST_BEHAVE/$me"; }
if [ ! -f "$file" ]; then
  case "$kind" in
    decide) out="I'd pick B." ;;
    review) out="Reviewed." ;;
    other) out="Done." ;;
    *) out="Here is where things stand." ;;
  esac
  printf '{"type":"result","is_error":false,"num_turns":1,"result":"%s"}\n' "$out"
  exit 0
fi
first=$(head -n 1 "$file")
case "$first" in
  WAIT:*)
    release="${first#WAIT:}"
    i=0
    while [ ! -f "$release" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
    out=$(tail -n +2 "$file" | sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g') ;;
  *) out=$(sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g' "$file") ;;
esac
printf '{"type":"result","is_error":false,"num_turns":1,"result":"%s"}\n' "$out"
"#;

struct Dirs {
    prompts: PathBuf,
    behave: PathBuf,
    root: PathBuf,
}

fn dirs() -> &'static Dirs {
    static DIRS: OnceLock<Dirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let (bin, prompts, behave) = (root.join("bin"), root.join("prompts"), root.join("behave"));
        for d in [&bin, &prompts, &behave] {
            std::fs::create_dir_all(d).unwrap();
        }
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("KWNT_TEST_BEHAVE", &behave);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        Dirs { prompts, behave, root }
    })
}

/// What bot `name` replies on a turn of `kind` ("" for `other`); names are unique per test.
fn behave(name: &str, kind: &str, reply: &str) {
    let file = if kind.is_empty() { name.to_string() } else { format!("{name}.{kind}") };
    std::fs::write(dirs().behave.join(file), reply).unwrap();
}

/// A file a "WAIT:" turn waits for.
fn release_file(name: &str) -> PathBuf {
    dirs().root.join(name)
}

/// Prompts bot `name` got on turns of `kind`, oldest first.
fn prompts_of(name: &str, kind: &str) -> Vec<String> {
    let prefix = format!("{name}.{kind}-");
    let mut found: Vec<(u128, String)> = std::fs::read_dir(&dirs().prompts)
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let ns = e.file_name().into_string().ok()?.strip_prefix(&prefix)?.parse().ok()?;
            Some((ns, std::fs::read_to_string(e.path()).ok()?))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// Every prompt bot `name` got, of any kind.
fn all_prompts_of(name: &str) -> usize {
    std::fs::read_dir(&dirs().prompts)
        .unwrap()
        .filter(|e| e.as_ref().ok().and_then(|e| e.file_name().into_string().ok()).is_some_and(|f| f.starts_with(&format!("{name}."))))
        .count()
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
    _tmp: tempfile::TempDir,
}

/// Keep-going turns stay on (their default): they are what a Stop must not set off. `board`
/// turns the mission board on (hand-off cards, the lead's review turns and the sweep).
async fn setup(board: bool) -> Env {
    dirs();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    db.set_setting("mission_board", if board { "true" } else { "false" }).await.unwrap();
    // The lead's review is what the board tests cover; the Swarm Auditor holds review during an audit
    db.set_setting("auditor", "off").await.unwrap();
    // Laya is unreachable: turns use the kwant's preferred model, the fake claude
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    if board {
        // The board follows hand-offs on the event bus, as the scheduler starts it in kwnt
        missions::listener::spawn(state.clone(), tokio_util::sync::CancellationToken::new());
    }
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

async fn add_bot(env: &Env, name: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/kwants", Some(json!({"name": name, "title": "Specialist", "persona_prompt": "", "soul": "", "avatar_sprite": "default", "driver_type": "cli", "preferred_model": "claude"}))).await;
    assert!(s.is_success(), "{s} {v}");
    v["id"].as_str().or(v["kwant"]["id"].as_str()).unwrap().to_string()
}

/// A team led by the first of `names` with all of them in it; returns the channel and the bot ids.
async fn team(env: &Env, team: &str, names: &[&str]) -> (String, Vec<String>) {
    let mut ids = Vec::new();
    for n in names {
        ids.push(add_bot(env, n).await);
    }
    let (s, v) = call(&env.app, "POST", "/api/teams", Some(json!({"name": team, "lead_kwant_id": ids[0], "member_kwants": ids}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, channels) = call(&env.app, "GET", "/api/channels", None).await;
    let channel = channels.as_array().unwrap().iter().find(|c| c["name"] == format!("team-{team}").as_str()).unwrap()["id"].as_str().unwrap().to_string();
    (channel, ids)
}

async fn say_in(env: &Env, channel: &str, prompt: &str) {
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn messages(env: &Env, channel: &str) -> Vec<Message> {
    env.state.db.list_messages(channel, 200).await.unwrap()
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
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

/// What still runs for `channel`: a turn, a job holding the chat, or one of the chat's hand-offs
/// that runs or whose job is held.
/// - The person's message holds a job keyed by the chat until its turn and the hand-offs it
///   starts are out (routes.rs `send_chat`). A lead resumed with results holds the chat until its
///   review turn, keep-going turns and the card they may open are done: they run inside that job
///   (delegation.rs `continue_lead`, then `keep_going::after_lead_turn`).
/// - A hand-off runs in a job keyed by its own id (delegation.rs `spawn_run`). That job ends with
///   the teammate's turn, before `finish` saves the result and resumes the lead, so a test whose
///   chain has a hand-off to finish waits for the delegation rows or the lead's resume first.
async fn running(env: &Env, channel: &str) -> Vec<String> {
    let turns = &env.state.turns;
    let mut what: Vec<String> = turns.active_turns().iter().map(|t| format!("{} turn of {} in {}", t.state, t.kwant_id, t.context_id)).collect();
    if turns.has_job(channel) {
        what.push(format!("a job holding the chat: {:?}", turns.job_keys_holding(channel)));
    }
    for d in env.state.db.list_delegations(None, None, 200).await.unwrap() {
        if d.origin_channel_id.as_deref() == Some(channel) && (d.status == "running" || turns.has_job(&d.id)) {
            what.push(format!("the hand-off to {} ({}): {:?}", d.specialist_kwant_id, d.status, turns.job_keys_holding(&d.id)));
        }
    }
    what
}

/// Waits until the chat's work has ended, then checks nothing starts there again.
async fn settle(env: &Env, channel: &str) {
    wait_for("the chat's work to end", || async { running(env, channel).await.is_empty() }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let late = running(env, channel).await;
    assert!(late.is_empty(), "started in the chat after its work ended: {late:?}");
}

async fn delegation_to(env: &Env, lead: &str, to: &str) -> Option<kwnt::db::threads::Delegation> {
    env.state.db.list_delegations(Some(lead), None, 20).await.unwrap().into_iter().find(|d| d.specialist_kwant_id == to)
}

async fn status_to(env: &Env, lead: &str, to: &str) -> String {
    delegation_to(env, lead, to).await.map(|d| d.status).unwrap_or_default()
}

/// Nothing restarted the work in `channel` after the Stop: no keep-going turn for `lead`, no
/// keep-going message or "Keep going?" card, and no card left open for the person.
async fn nothing_restarted(env: &Env, channel: &str, lead: &str) {
    let kg = prompts_of(lead, "kg");
    assert!(kg.is_empty(), "the person stopped a teammate, yet {lead} got {} keep-going turn(s): {:?}", kg.len(), kg.first());
    let msgs = messages(env, channel).await;
    let restarted: Vec<&str> = msgs
        .iter()
        .filter(|m| meta(m).get(keep_going::TURN_KEY).is_some() || meta(m).get(keep_going::CARD_KEY).is_some() || m.text.contains("Keep going?"))
        .map(|m| m.text.as_str())
        .collect();
    assert!(restarted.is_empty(), "keep-going spoke after the Stop: {restarted:?}");
    let open = env.state.db.list_questions(Some("open"), Some(channel), None).await.unwrap();
    assert!(open.is_empty(), "a card waits on the person after the Stop: {:?}", open.iter().map(|q| &q.question).collect::<Vec<_>>());
}

/// (C) The lead hands out three tasks: Ck1 checks the sources, Rs1 researches, and Wr1 writes
/// once Rs1 has the facts. Ck1 finishes; the person stops Rs1 on its card. Wr1 is Stopped with
/// it (it never ran), and the lead's one results turn hears "It was stopped", not "It failed".
/// After that turn nothing restarts the work: before 32h keep-going counted the stopped hand-off
/// as finished, ran its turns ("Do the next step now") and asked "… Keep going?".
#[tokio::test]
async fn stopping_a_hand_off_another_waits_for_stops_both_and_nothing_restarts_them() {
    let env = setup(false).await;
    let (channel, ids) = team(&env, "stop-c", &["Ld1", "Ck1", "Rs1", "Wr1"]).await;
    let (lead, ck, rs, wr) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    behave("Ld1", "", "On it. @Ck1 please check the sources. @Wr1, once @Rs1 has the facts, write the draft. @Rs1 please research the facts.");
    behave("Ld1", "res", "Ck1 checked the sources. Rs1 was stopped, so there is no draft yet.");
    behave("Ld1", "kg", "Doing the next step now: researching the facts myself.");
    behave("Ck1", "", "Sources checked.");
    behave("Rs1", "", &format!("WAIT:{}\nFacts found.", release_file("rs1-never").display()));
    behave("Wr1", "", "Draft written.");
    say_in(&env, &channel, "@Ld1 write the report").await;

    wait_for("Ck1's result and Rs1 at work", || async {
        status_to(&env, lead, ck).await == "completed" && status_to(&env, lead, rs).await == "running"
    })
    .await;
    assert_eq!(status_to(&env, lead, wr).await, "queued", "Wr1 waits for Rs1");

    // The person presses Stop on Rs1's card
    let rs_id = delegation_to(&env, lead, rs).await.unwrap().id;
    let (s, v) = call(&env.app, "POST", &format!("/api/delegations/{rs_id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("Wr1's task to end", || async { delegation_to(&env, lead, wr).await.is_some_and(|d| !d.is_open()) }).await;
    wait_for("the lead's results turn", || async { !prompts_of("Ld1", "res").is_empty() }).await;
    settle(&env, &channel).await;

    nothing_restarted(&env, &channel, "Ld1").await;
    assert_eq!(prompts_of("Ld1", "res").len(), 1, "one results turn for the batch");

    let w = delegation_to(&env, lead, wr).await.unwrap();
    assert_eq!(w.status, "cancelled", "Wr1 never ran because the person stopped Rs1: it is Stopped, not Failed ({w:?})");
    assert!(w.error.as_deref().is_some_and(|e| e.contains("Rs1's task was stopped")), "{w:?}");
    assert_eq!(all_prompts_of("Wr1"), 0, "Wr1 never ran");
    assert_eq!(status_to(&env, lead, rs).await, "cancelled");

    let res = &prompts_of("Ld1", "res")[0];
    assert!(res.contains("Sources checked."), "the lead hears Ck1's result: {res}");
    assert!(!res.contains("It failed."), "nothing in the batch failed: {res}");
    assert!(res.matches("It was stopped. Don't restart it unless the person asks.").count() >= 2, "Rs1 and Wr1 read as stopped: {res}");

    let msgs = messages(&env, &channel).await;
    assert!(
        msgs.iter().any(|m| m.text.contains("Wr1 didn't start") && m.text.contains("Rs1's task was stopped")),
        "the chat says why Wr1 didn't start: {:?}",
        msgs.iter().map(|m| &m.text).collect::<Vec<_>>()
    );
}

/// (C) The same with no other hand-off: stopping Rs2 stops Wr2, which waits for it, and the batch
/// is all stopped, so the lead isn't resumed at all (as for a single stopped hand-off).
#[tokio::test]
async fn stopping_the_only_running_hand_off_leaves_the_lead_quiet() {
    let env = setup(false).await;
    let (channel, ids) = team(&env, "stop-c2", &["Ld2", "Rs2", "Wr2"]).await;
    let (lead, rs, wr) = (&ids[0], &ids[1], &ids[2]);
    behave("Ld2", "", "On it. @Wr2, once @Rs2 has the facts, write the draft. @Rs2 please research the facts.");
    behave("Ld2", "res", "Rs2 was stopped, so there is no draft yet.");
    behave("Ld2", "kg", "Doing the next step now: researching the facts myself.");
    behave("Rs2", "", &format!("WAIT:{}\nFacts found.", release_file("rs2-never").display()));
    say_in(&env, &channel, "@Ld2 write the report").await;

    wait_for("Rs2 at work", || async { status_to(&env, lead, rs).await == "running" }).await;
    let rs_id = delegation_to(&env, lead, rs).await.unwrap().id;
    let (s, v) = call(&env.app, "POST", &format!("/api/delegations/{rs_id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("Wr2's task to end", || async { delegation_to(&env, lead, wr).await.is_some_and(|d| !d.is_open()) }).await;
    settle(&env, &channel).await;

    nothing_restarted(&env, &channel, "Ld2").await;
    assert_eq!(status_to(&env, lead, wr).await, "cancelled", "Wr2 is Stopped with Rs2, not Failed");
    assert!(prompts_of("Ld2", "res").is_empty(), "everything the lead handed out was stopped: it isn't resumed");
    assert_eq!(all_prompts_of("Wr2"), 0, "Wr2 never ran");
}

/// The hand-off of `lead` to `to` finished and its card reached Review.
async fn card_in_review(env: &Env, lead: &str, to: &str) -> Option<kwnt::db::missions::Task> {
    let d = delegation_to(env, lead, to).await.filter(|d| d.status == "completed")?;
    env.state.db.task_by_source("delegation", &d.id).await.unwrap().filter(|t| t.state == "review")
}

/// (S) The person stops the lead while it writes the summary of its hand-off's result: no review
/// turn runs after it (before 32h the fresh-eyes review posted "Both pass: …" anyway), the sweep
/// for lost reviews doesn't bring one back, and no other turn starts. The card waits in Review
/// for the person.
#[tokio::test]
async fn stopping_the_leads_summary_runs_no_review_turn() {
    let env = setup(true).await;
    let (channel, ids) = team(&env, "stop-s", &["Ld3", "Ck3"]).await;
    let (lead, ck) = (&ids[0], &ids[1]);
    let release = release_file("ld3-summary");
    behave("Ld3", "", "On it. @Ck3 please check the parser.");
    behave("Ld3", "res", &format!("WAIT:{}\nCk3 checked the parser SUMMARK3.", release.display()));
    behave("Ck3", "", "Parser checked.");
    say_in(&env, &channel, "@Ld3 check the parser").await;

    wait_for("the card in Review", || async { card_in_review(&env, lead, ck).await.is_some() }).await;
    wait_for("the lead's summary turn", || async {
        !prompts_of("Ld3", "res").is_empty() && env.state.turns.active_turns().iter().any(|t| &t.kwant_id == lead)
    })
    .await;

    // The person presses Stop on the chat while the lead writes the summary
    let (s, v) = call(&env.app, "POST", "/api/chat/cancel", Some(json!({"channel_id": channel}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(&env, &channel).await;
    // The sweep for cards whose review was lost (after a restart) leaves a stopped one alone
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle(&env, &channel).await;
    // 32o: kwnt restarts (what it kept in memory is gone) and the sweep runs at once: the Stop
    // still holds, read from the card
    kwnt::server::delegation::forget_stopped_reviews();
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle(&env, &channel).await;
    std::fs::write(&release, "").unwrap();

    let reviews = prompts_of("Ld3", "review");
    assert!(reviews.is_empty(), "the person stopped the summary, yet a review turn ran: {:?}", reviews.first());
    nothing_restarted(&env, &channel, "Ld3").await;
    assert_eq!(prompts_of("Ld3", "res").len(), 1, "the stopped summary isn't run again");
    assert!(card_in_review(&env, lead, ck).await.is_some(), "the card waits in Review for the person");
    assert!(!messages(&env, &channel).await.iter().any(|m| m.text.contains("SUMMARK3")), "the stopped summary isn't posted");
}

/// (S, 32o) After the person stopped the lead's summary and kwnt restarted, the sweep leaves the
/// card alone. Then the card is sent back and its fix ends in Review again: that is a new stay, the
/// old Stop no longer holds, and the sweep asks the lead to review it (once, then one reminder at
/// most).
#[tokio::test]
async fn a_card_back_in_review_after_a_stopped_summary_is_reviewed_again() {
    let env = setup(true).await;
    let (channel, ids) = team(&env, "stop-s5", &["Ld5", "Ck5"]).await;
    let (lead, ck) = (&ids[0], &ids[1]);
    let release = release_file("ld5-summary");
    behave("Ld5", "", "On it. @Ck5 please check the parser.");
    behave("Ld5", "res", &format!("WAIT:{}\nCk5 checked the parser SUMMARK5.", release.display()));
    behave("Ck5", "", "Parser checked.");
    say_in(&env, &channel, "@Ld5 check the parser").await;

    wait_for("the card in Review", || async { card_in_review(&env, lead, ck).await.is_some() }).await;
    wait_for("the lead's summary turn", || async {
        !prompts_of("Ld5", "res").is_empty() && env.state.turns.active_turns().iter().any(|t| &t.kwant_id == lead)
    })
    .await;
    let (s, v) = call(&env.app, "POST", "/api/chat/cancel", Some(json!({"channel_id": channel}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(&env, &channel).await;
    std::fs::write(&release, "").unwrap();

    // kwnt restarts, then the sweep: the Stop holds
    kwnt::server::delegation::forget_stopped_reviews();
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle(&env, &channel).await;
    assert!(prompts_of("Ld5", "review").is_empty(), "a review turn ran after the Stop and a restart: {:?}", prompts_of("Ld5", "review").first());
    let card = card_in_review(&env, lead, ck).await.expect("the card waits in Review");
    let events = env.state.db.task_events(&card.id, 500).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "lead_review_stopped"), "the Stop is kept on the card: {:?}", events.iter().map(|e| &e.kind).collect::<Vec<_>>());

    // The card was sent back and its fix ended in Review again: its moves as the card records
    // them (straight in the database, so no board path asks the lead on its own and only the
    // sweep can)
    for (from, to) in [("review", "ready"), ("ready", "review")] {
        let change = kwnt::db::missions::TaskChange { kind: "state".into(), reason: Some("sent back and fixed".into()), ..Default::default() };
        assert!(env.state.db.move_task(&card.id, from, to, "kwant", &change).await.unwrap(), "{from} -> {to}");
    }
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    wait_for("the lead's review turn", || async { !prompts_of("Ld5", "review").is_empty() }).await;
    settle(&env, &channel).await;
    // The asks are capped per stay: another sweep starts nothing more
    missions::listener::sweep_unreviewed(&env.state, Duration::ZERO).await;
    settle(&env, &channel).await;

    let reviews = prompts_of("Ld5", "review");
    assert!(
        (1..=missions::listener::MAX_REVIEW_ASKS).contains(&reviews.len()),
        "the sweep asks once (and reminds once at most): {} review turns",
        reviews.len()
    );
    assert!(reviews[0].contains("Parser checked."), "the review sees the result: {}", reviews[0]);
    assert_eq!(prompts_of("Ld5", "res").len(), 1, "the stopped summary isn't run again");
    assert!(prompts_of("Ld5", "kg").is_empty(), "no keep-going turn");
    assert!(card_in_review(&env, lead, ck).await.is_some(), "the fake lead never calls task_review: the card stays in Review");
}

/// (A) No Stop: the lead's summary is posted and its review turn runs after it, as before.
#[tokio::test]
async fn without_a_stop_the_summary_and_its_review_run() {
    let env = setup(true).await;
    let (channel, ids) = team(&env, "stop-a", &["Ld4", "Ck4"]).await;
    let (lead, ck) = (&ids[0], &ids[1]);
    behave("Ld4", "", "On it. @Ck4 please check the parser.");
    behave("Ld4", "res", "Ck4 checked the parser SUMMARK4.\\nDone: checked the parser against the brief.");
    behave("Ck4", "", "Parser checked.");
    say_in(&env, &channel, "@Ld4 check the parser").await;

    wait_for("the lead's review turn", || async { !prompts_of("Ld4", "review").is_empty() }).await;
    settle(&env, &channel).await;

    assert_eq!(prompts_of("Ld4", "res").len(), 1, "one results turn");
    let review = &prompts_of("Ld4", "review")[0];
    assert!(review.contains("Parser checked."), "the review sees the result: {review}");
    assert!(messages(&env, &channel).await.iter().any(|m| m.text.contains("SUMMARK4")), "the summary is posted");
    assert!(prompts_of("Ld4", "kg").is_empty(), "a Done line needs no keep-going turn");
    assert!(card_in_review(&env, lead, ck).await.is_some(), "the fake lead never calls task_review: the card stays in Review");
}

/// Whether `lead`'s own turn still runs (it holds the hand-offs it started until it ends).
fn lead_at_work(env: &Env, lead: &str) -> bool {
    env.state.turns.active_turns().iter().any(|t| t.kwant_id == lead)
}

/// 33s: the lead's Ck result is reported in the moment after a task ended and before what waits
/// for it moved on: `pause` (from `pause_release`, armed before that task ended) holds that
/// moment, `ck_go` lets Ck reply. Waits until Ck's report ran (its run is over) or the lead was
/// already resumed, then lets the release go on.
async fn report_ck_while_releasing(env: &Env, lead: &str, ck: &str, pause: Pause, ck_go: &str) {
    let (reached, go) = pause;
    reached.notified().await;
    let ck_id = delegation_to(env, lead, ck).await.unwrap().id;
    std::fs::write(release_file(ck_go), "").unwrap();
    let lead_name = env.state.db.get_kwant(lead).await.unwrap().unwrap().name;
    wait_for("Ck's report", || async {
        (status_to(env, lead, ck).await == "completed" && !env.state.turns.has_job(&ck_id)) || !prompts_of(&lead_name, "res").is_empty()
    })
    .await;
    go.notify_one();
}

type Pause = (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);

/// 33s: (C) made certain. Ck6's result is reported in the moment after the person stopped Rs6 and
/// before Wr6, parked behind it, is stopped too. The lead still gets one results turn, which
/// hears all three. Before 33s Ck6's report took the batch without Wr6 (a parked task doesn't
/// count), the Stop's report a moment later took it with Wr6, and the lead was resumed twice
/// (CI on #1006 and #992: "one results turn for the batch, left: 2, right: 1").
#[tokio::test]
async fn a_result_reported_while_a_stop_ends_the_task_parked_behind_it_resumes_the_lead_once() {
    let env = setup(false).await;
    let (channel, ids) = team(&env, "stop-r", &["Ld6", "Ck6", "Rs6", "Wr6"]).await;
    let (lead, ck, rs, wr) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    behave("Ld6", "", "On it. @Ck6 please check the sources. @Wr6, once @Rs6 has the facts, write the draft. @Rs6 please research the facts.");
    behave("Ld6", "res", "Ck6 checked the sources. Rs6 was stopped, so there is no draft yet.");
    behave("Ld6", "kg", "Doing the next step now: researching the facts myself.");
    behave("Ck6", "", &format!("WAIT:{}\nSources checked.", release_file("ck6-go").display()));
    behave("Rs6", "", &format!("WAIT:{}\nFacts found.", release_file("rs6-never").display()));
    behave("Wr6", "", "Draft written.");
    say_in(&env, &channel, "@Ld6 write the report").await;

    wait_for("Ck6 and Rs6 at work, the lead's turn over", || async {
        status_to(&env, lead, ck).await == "running" && status_to(&env, lead, rs).await == "running" && !lead_at_work(&env, lead)
    })
    .await;
    assert_eq!(status_to(&env, lead, wr).await, "queued", "Wr6 waits for Rs6");
    let rs_id = delegation_to(&env, lead, rs).await.unwrap().id;
    let pause = kwnt::server::delegation::pause_release(&rs_id);
    let (app, uri) = (env.app.clone(), format!("/api/delegations/{rs_id}/cancel"));
    let stop = tokio::spawn(async move { call(&app, "POST", &uri, None).await });
    // The Stop pauses right after Rs6 is saved as stopped
    wait_for("Rs6 stopped", || async { status_to(&env, lead, rs).await == "cancelled" }).await;
    assert_eq!(status_to(&env, lead, wr).await, "queued", "Wr6 is still parked in that moment");
    report_ck_while_releasing(&env, lead, ck, pause, "ck6-go").await;
    let (s, v) = stop.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("Wr6's task to end", || async { delegation_to(&env, lead, wr).await.is_some_and(|d| !d.is_open()) }).await;
    wait_for("the lead's results turn", || async { !prompts_of("Ld6", "res").is_empty() }).await;
    settle(&env, &channel).await;

    assert_eq!(prompts_of("Ld6", "res").len(), 1, "one results turn for the batch");
    nothing_restarted(&env, &channel, "Ld6").await;
    let res = &prompts_of("Ld6", "res")[0];
    assert!(res.contains("Sources checked."), "the lead hears Ck6's result: {res}");
    assert!(res.matches("It was stopped. Don't restart it unless the person asks.").count() >= 2, "Rs6 and Wr6 read as stopped: {res}");
    assert_eq!(status_to(&env, lead, wr).await, "cancelled");
    assert_eq!(all_prompts_of("Wr6"), 0, "Wr6 never ran");
}

/// 33s: the same moment without a Stop. Rs7 finishes, and Ck7's result is reported before Wr7,
/// which waited for Rs7, starts. The lead hears all three once, after Wr7. Before 33s it was
/// resumed with Ck7's and Rs7's results, then again with all three once Wr7 ended.
#[tokio::test]
async fn a_result_reported_while_a_finished_task_starts_the_one_parked_behind_it_resumes_the_lead_once() {
    let env = setup(false).await;
    let (channel, ids) = team(&env, "stop-r2", &["Ld7", "Ck7", "Rs7", "Wr7"]).await;
    let (lead, ck, rs, wr) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    behave("Ld7", "", "On it. @Ck7 please check the sources. @Wr7, once @Rs7 has the facts, write the draft. @Rs7 please research the facts.");
    behave("Ld7", "res", "Here is the draft, with checked sources.");
    behave("Ck7", "", &format!("WAIT:{}\nSources checked.", release_file("ck7-go").display()));
    behave("Rs7", "", &format!("WAIT:{}\nFacts found.", release_file("rs7-go").display()));
    behave("Wr7", "", "Draft written.");
    say_in(&env, &channel, "@Ld7 write the report").await;

    wait_for("Ck7 and Rs7 at work, the lead's turn over", || async {
        status_to(&env, lead, ck).await == "running" && status_to(&env, lead, rs).await == "running" && !lead_at_work(&env, lead)
    })
    .await;
    let rs_id = delegation_to(&env, lead, rs).await.unwrap().id;
    let pause = kwnt::server::delegation::pause_release(&rs_id);
    std::fs::write(release_file("rs7-go"), "").unwrap();
    // Rs7's run pauses right after its result is saved
    wait_for("Rs7's result", || async { status_to(&env, lead, rs).await == "completed" }).await;
    assert_eq!(status_to(&env, lead, wr).await, "queued", "Wr7 is still parked in that moment");
    report_ck_while_releasing(&env, lead, ck, pause, "ck7-go").await;
    wait_for("Wr7's result", || async { status_to(&env, lead, wr).await == "completed" }).await;
    wait_for("the lead's results turn", || async { !prompts_of("Ld7", "res").is_empty() }).await;
    settle(&env, &channel).await;

    assert_eq!(prompts_of("Ld7", "res").len(), 1, "one results turn for the batch");
    let res = &prompts_of("Ld7", "res")[0];
    for r in ["Sources checked.", "Facts found.", "Draft written."] {
        assert!(res.contains(r), "the lead hears {r:?}: {res}");
    }
}

/// The rule of 2026-10-24 still holds around that moment: Rs8 and Ck8 end close together, Ck8's
/// report runs after Rs8's result is saved but before Rs8's own report. The lead is resumed
/// once, with both (not twice, and not never).
#[tokio::test]
async fn two_results_ending_close_together_resume_the_lead_once_with_both() {
    let env = setup(false).await;
    let (channel, ids) = team(&env, "stop-r3", &["Ld8", "Ck8", "Rs8"]).await;
    let (lead, ck, rs) = (&ids[0], &ids[1], &ids[2]);
    behave("Ld8", "", "On it. @Ck8 please check the sources. @Rs8 please research the facts.");
    behave("Ld8", "res", "Both are in.");
    behave("Ck8", "", &format!("WAIT:{}\nSources checked.", release_file("ck8-go").display()));
    behave("Rs8", "", &format!("WAIT:{}\nFacts found.", release_file("rs8-go").display()));
    say_in(&env, &channel, "@Ld8 write the report").await;

    wait_for("Ck8 and Rs8 at work, the lead's turn over", || async {
        status_to(&env, lead, ck).await == "running" && status_to(&env, lead, rs).await == "running" && !lead_at_work(&env, lead)
    })
    .await;
    let rs_id = delegation_to(&env, lead, rs).await.unwrap().id;
    let pause = kwnt::server::delegation::pause_release(&rs_id);
    std::fs::write(release_file("rs8-go"), "").unwrap();
    report_ck_while_releasing(&env, lead, ck, pause, "ck8-go").await;
    wait_for("the lead's results turn", || async { !prompts_of("Ld8", "res").is_empty() }).await;
    settle(&env, &channel).await;

    assert_eq!(prompts_of("Ld8", "res").len(), 1, "one results turn for the batch");
    let res = &prompts_of("Ld8", "res")[0];
    assert!(res.contains("Sources checked.") && res.contains("Facts found."), "the lead hears both: {res}");
}
