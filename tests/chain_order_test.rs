//! One reply that hands on to several bots (F4) and a chain where several bots ask the person
//! something (F5). A fake claude CLI on PATH answers as the bot named in its system prompt
//! ("You are <name>, …"), from files under `$KWNT_TEST_BEHAVE`: `<name>` for a turn, `<name>.list`
//! for the turn that gathers the person's questions and `<name>.resume` for a lead resumed with
//! results. A first line "FAIL" fails the turn and "WAIT:<file>" holds it until that file exists.
//! Every prompt is saved as `$KWNT_TEST_PROMPTS/<name>-<ns>`. Its own test binary: it changes PATH.

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
    server::{create_router, AppState},
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
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$me-$(date +%s%N)"
file="$KWNT_TEST_BEHAVE/$me"
case "$last" in
  *"as one numbered list"*) [ -f "$file.list" ] && file="$file.list" ;;
  *"finished the task you delegated"*|*"tasks you delegated have ended"*) [ -f "$file.resume" ] && file="$file.resume" ;;
esac
if [ ! -f "$file" ]; then
  echo '{"type":"result","is_error":false,"num_turns":1,"result":"Done."}'
  exit 0
fi
first=$(head -n 1 "$file")
case "$first" in
  FAIL) echo "boom" >&2; exit 1 ;;
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
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("KWNT_TEST_BEHAVE", &behave);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        Dirs { prompts, behave, root }
    })
}

/// What bot `name` replies (see the module doc); names are unique per test.
fn behave(name: &str, reply: &str) {
    std::fs::write(dirs().behave.join(name), reply).unwrap();
}

/// A file a "WAIT:" turn waits for.
fn release_file(name: &str) -> PathBuf {
    dirs().root.join(name)
}

/// Prompts bot `name` got, oldest first.
fn prompts_of(name: &str) -> Vec<String> {
    let mut found: Vec<(u128, String)> = std::fs::read_dir(&dirs().prompts)
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let file = e.file_name().into_string().ok()?;
            let ns = file.strip_prefix(&format!("{name}-"))?.parse().ok()?;
            Some((ns, std::fs::read_to_string(e.path()).ok()?))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// When bot `name` got its first prompt (ns).
fn first_prompt_at(name: &str) -> Option<u128> {
    std::fs::read_dir(&dirs().prompts)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_prefix(&format!("{name}-"))?.parse().ok())
        .min()
}

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

struct Env {
    app: Router,
    state: AppState,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    dirs();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // These turns predate the mission board, which is now on by default; keep it off here.
    db.set_setting("mission_board", "false").await.unwrap();
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

/// Waits until nothing runs in this test's chats any more.
async fn settle(env: &Env) {
    let mut quiet = 0;
    for _ in 0..400 {
        quiet = if env.state.turns.active_turns().is_empty() { quiet + 1 } else { 0 };
        if quiet >= 20 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the chain never went quiet");
}

async fn delegation_to(env: &Env, lead: &str, to: &str) -> Option<kwnt::db::threads::Delegation> {
    env.state.db.list_delegations(Some(lead), None, 20).await.unwrap().into_iter().find(|d| d.specialist_kwant_id == to)
}

// --- F4: handoffs (a teammate who isn't the lead hands on) ---

#[tokio::test]
async fn a_handoff_that_waits_for_another_gets_its_result() {
    let env = setup().await;
    let (channel, _) = team(&env, "f4-hand", &["Ld1", "Hub1", "Wr1", "An1"]).await;
    behave("Hub1", "Splitting it up. @Wr1, once @An1 has the numbers, write the summary. @An1 please crunch the numbers.");
    behave("An1", "Revenue is 42k ANRESULT1.");
    behave("Wr1", "Summary written.");
    say_in(&env, &channel, "@Hub1 plan the report").await;
    wait_for("Wr1's turn", || async { !prompts_of("Wr1").is_empty() }).await;
    settle(&env).await;

    assert!(first_prompt_at("An1") < first_prompt_at("Wr1"), "An1, mentioned second, runs first");
    let p = &prompts_of("Wr1")[0];
    assert!(p.contains("asked you to wait for An1's result") && p.contains("ANRESULT1"), "Wr1 gets An1's result: {p}");
    assert_eq!(prompts_of("An1").len(), 1);
}

#[tokio::test]
async fn a_handoff_waiting_for_a_failed_or_asking_teammate_does_not_start() {
    let env = setup().await;
    let (channel, _) = team(&env, "f4-fail", &["Ld2", "Hub2", "Wr2", "An2", "Sc2"]).await;
    behave("Hub2", "Splitting it up. @Wr2, once @An2 has the numbers, write the summary. @An2 please crunch the numbers. @Sc2 please scan the market.");
    behave("An2", "FAIL");
    behave("Sc2", "Market scanned.");
    say_in(&env, &channel, "@Hub2 plan the report").await;
    wait_for("Sc2's turn", || async { !prompts_of("Sc2").is_empty() }).await;
    settle(&env).await;
    assert!(prompts_of("Wr2").is_empty(), "Wr2 waited for An2, which failed");
    let msgs = messages(&env, &channel).await;
    assert!(msgs.iter().any(|m| m.text.contains("Wr2") && m.text.contains("An2")), "the chain says Wr2 didn't start: {:?}", msgs.iter().map(|m| &m.text).collect::<Vec<_>>());

    // A prerequisite that asks the person something holds its dependent too
    let (channel, _) = team(&env, "f4-ask", &["Ld7", "Hub7", "Wr7", "An7"]).await;
    behave("Hub7", "Splitting it up. @Wr7, once @An7 has the numbers, write the summary. @An7 please crunch the numbers.");
    behave("An7", "I can pull them.\nWhich quarter should I use?");
    say_in(&env, &channel, "@Hub7 plan the report").await;
    wait_for("An7's turn", || async { !prompts_of("An7").is_empty() }).await;
    settle(&env).await;
    assert!(prompts_of("Wr7").is_empty(), "Wr7 waits for the person's answer to An7");
}

// --- F4: a lead's delegations ---

#[tokio::test]
async fn a_leads_dependent_delegation_waits_while_independent_ones_run_in_parallel() {
    let env = setup().await;
    let (channel, ids) = team(&env, "f4-lead", &["Ld3", "Wr3", "An3", "Sc3"]).await;
    let (lead, wr, an, sc) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    let release = release_file("an3-release");
    behave("Ld3", "On it. @Wr3, once @An3 has the numbers, write the summary. @An3 please crunch the numbers. @Sc3 please scan the market.");
    behave("Ld3.resume", "All three parts are in.");
    behave("An3", &format!("WAIT:{}\nRevenue is 42k ANRESULT3.", release.display()));
    behave("Sc3", "Market scanned.");
    behave("Wr3", "Summary written WRRESULT3.");
    say_in(&env, &channel, "@Ld3 get me the report").await;

    // Sc3 finishes while An3 still works: independent mentions run at once
    wait_for("Sc3's result", || async { delegation_to(&env, lead, sc).await.is_some_and(|d| d.status == "completed") }).await;
    let a = delegation_to(&env, lead, an).await.unwrap();
    assert!(a.is_open(), "An3 still works: {a:?}");
    let w = delegation_to(&env, lead, wr).await.expect("Wr3's task exists");
    assert_eq!(w.status, "queued");
    assert_eq!(w.after_delegation_id.as_deref(), Some(a.id.as_str()));
    assert!(prompts_of("Wr3").is_empty(), "Wr3 hasn't started");
    assert!(prompts_of("Ld3").iter().all(|p| !p.contains("tasks you delegated have ended")), "the lead isn't resumed while Wr3 waits");

    std::fs::write(&release, "").unwrap();
    wait_for("Wr3's result", || async { delegation_to(&env, lead, wr).await.is_some_and(|d| d.status == "completed") }).await;
    let p = &prompts_of("Wr3")[0];
    assert!(p.contains("ANRESULT3"), "Wr3 gets An3's result: {p}");
    wait_for("the lead's resume", || async { prompts_of("Ld3").iter().any(|p| p.contains("WRRESULT3")) }).await;
    settle(&env).await;
    let resumes: Vec<_> = prompts_of("Ld3").into_iter().filter(|p| p.contains("WRRESULT3")).collect();
    assert_eq!(resumes.len(), 1, "one resume with every result");
    assert!(resumes[0].contains("ANRESULT3"), "{}", resumes[0]);
}

#[tokio::test]
async fn a_leads_delegation_waiting_for_a_failed_or_asking_one_does_not_start() {
    let env = setup().await;
    let (channel, ids) = team(&env, "f4-lead-fail", &["Ld4", "Wr4", "An4"]).await;
    behave("Ld4", "On it. @Wr4, once @An4 has the numbers, write the summary. @An4 please crunch the numbers.");
    behave("Ld4.resume", "An4 failed, so nothing was written.");
    behave("An4", "FAIL");
    say_in(&env, &channel, "@Ld4 get me the report").await;
    wait_for("Wr4's task to end", || async { delegation_to(&env, &ids[0], &ids[1]).await.is_some_and(|d| !d.is_open()) }).await;
    settle(&env).await;
    let w = delegation_to(&env, &ids[0], &ids[1]).await.unwrap();
    assert_eq!(w.status, "failed");
    assert!(prompts_of("Wr4").is_empty(), "Wr4 never ran");
    assert!(messages(&env, &channel).await.iter().any(|m| m.text.contains("Wr4 didn't start")), "the channel says so");

    let (channel, ids) = team(&env, "f4-lead-ask", &["Ld5", "Wr5", "An5"]).await;
    behave("Ld5", "On it. @Wr5, once @An5 has the numbers, write the summary. @An5 please crunch the numbers.");
    behave("An5", "I can pull them.\nWhich quarter should I use?");
    say_in(&env, &channel, "@Ld5 get me the report").await;
    wait_for("Wr5's task to end", || async { delegation_to(&env, &ids[0], &ids[1]).await.is_some_and(|d| !d.is_open()) }).await;
    settle(&env).await;
    assert!(prompts_of("Wr5").is_empty(), "Wr5 never ran");
    let w = delegation_to(&env, &ids[0], &ids[1]).await.unwrap();
    assert!(w.error.as_deref().is_some_and(|e| e.contains("asked you something first")), "{w:?}");
}

// --- F5: one list of what the team needs from the person ---

#[tokio::test]
async fn two_askers_get_one_list_from_the_lead_and_the_answer_goes_to_it() {
    let env = setup().await;
    let (channel, _) = team(&env, "f5-two", &["Ld6", "Hub6", "As6", "Bs6"]).await;
    behave("Hub6", "Splitting it. @As6 please check the dates. @Bs6 please check the venues.");
    behave("As6", "Dates look tight.\nWhich month do you prefer?");
    behave("Bs6", "Venues found.\nWhat budget should I use?");
    behave("Ld6.list", "To go on we need:\n1. As6: which month\n2. Bs6: the budget");
    behave("Ld6", "Thanks, passing it on.");
    say_in(&env, &channel, "@Hub6 plan the offsite").await;
    wait_for("the list", || async { messages(&env, &channel).await.iter().any(|m| meta(m).get("consolidated_asks").is_some()) }).await;
    settle(&env).await;

    let gathering: Vec<_> = prompts_of("Ld6").into_iter().filter(|p| p.contains("as one numbered list")).collect();
    assert_eq!(gathering.len(), 1, "one extra turn");
    assert!(gathering[0].contains("Which month") && gathering[0].contains("What budget"), "{}", gathering[0]);
    let msgs = messages(&env, &channel).await;
    let list = msgs.iter().find(|m| meta(m).get("consolidated_asks").is_some()).unwrap();
    assert_eq!(list.sender_id, "Ld6");
    for asker in ["As6", "Bs6"] {
        let m = msgs.iter().find(|m| m.sender_id == asker).unwrap();
        assert_eq!(meta(m)["consolidated_into"], json!(list.id), "{asker}'s question links to the list");
    }

    let before = (prompts_of("As6").len(), prompts_of("Bs6").len());
    say_in(&env, &channel, "May, and 5000 EUR ANSWER6").await;
    wait_for("the lead's turn with the answer", || async { prompts_of("Ld6").iter().any(|p| p.contains("ANSWER6")) }).await;
    settle(&env).await;
    assert_eq!((prompts_of("As6").len(), prompts_of("Bs6").len()), before, "the answer went to the lead, not an asker");
    let p = prompts_of("Ld6").into_iter().find(|p| p.contains("ANSWER6")).unwrap();
    assert!(p.contains("Pass the part that answers") && p.contains("As6, Bs6"), "the lead is told whom to pass it to: {p}");
    assert_eq!(prompts_of("Ld6").iter().filter(|p| p.contains("as one numbered list")).count(), 1, "no second list");
}

#[tokio::test]
async fn one_asker_or_none_gets_no_extra_turn() {
    let env = setup().await;
    let (channel, _) = team(&env, "f5-one", &["Ld8", "Hub8", "As8", "Bs8"]).await;
    behave("Hub8", "Splitting it. @As8 please check the dates. @Bs8 please check the venues.");
    behave("As8", "Dates look tight.\nWhich month do you prefer?");
    behave("Bs8", "Venues found: three options.");
    say_in(&env, &channel, "@Hub8 plan the offsite").await;
    wait_for("Bs8's turn", || async { !prompts_of("Bs8").is_empty() }).await;
    settle(&env).await;
    assert!(prompts_of("Ld8").iter().all(|p| !p.contains("as one numbered list")), "one asker: no list");
    assert!(messages(&env, &channel).await.iter().all(|m| meta(m).get("consolidated_asks").is_none()));

    let (channel, _) = team(&env, "f5-none", &["Ld9", "Hub9", "As9"]).await;
    behave("Hub9", "Splitting it. @As9 please check the dates.");
    behave("As9", "Dates are fine.");
    say_in(&env, &channel, "@Hub9 plan the offsite").await;
    wait_for("As9's turn", || async { !prompts_of("As9").is_empty() }).await;
    settle(&env).await;
    assert!(prompts_of("Ld9").is_empty(), "nobody asked: the lead isn't woken");
}
