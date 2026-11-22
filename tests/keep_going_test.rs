//! Keep going (0.9.2): a lead's chain never just stops after its results turn. A scripted fake
//! claude CLI replies from per-test queue files, so each test says what the lead answers on each
//! results turn and each keep-going turn. Its own test binary: it changes PATH.

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
    server::{create_router, delegation, keep_going, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// The person's message carries "SCRIPT:<dir>" (and delegation titles repeat it, so the results
/// prompt has it too). A prompt is a keep-going turn ("isn't marked done", queue `<dir>/kg`), a
/// lead's results turn ("finished the task you delegated" / "tasks you delegated have ended",
/// queue `<dir>/res`), or anything else (the lead's first turn, a teammate's task, a turn on an
/// answer), which replies "ok" (or the next line of queue `<dir>/other`, 33d); a teammate's task with "SPECHOLD:<file>" waits for that file
/// first. Each queue line is one reply (`\n` for a line break); an empty queue replies "Here is
/// where things stand.". A line "HOLD:<file> <reply>" waits for the file, then replies; a line
/// "FAILED:<reply>" runs a Bash step that fails first, then replies; "TFAILED:<reply>" runs a
/// `run_terminal_command` step that fails ("Error: no terminal session is open"), then replies
/// (22j, Quant's step); "TOK:<reply>" runs that step and it works, then replies. A follow-through continuation ("…failed or was refused, and your reply
/// ends with a plan…") reads queue `<dir>/ft`. Every prompt is saved as `<dir>/prompts/<kind>-…`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
dir=$(printf '%s\n' "$last" | sed -n 's/.*SCRIPT:\([^ ]*\).*/\1/p' | head -n 1)
kind=other
case "$last" in
  *"failed or was refused, and your reply ends with a plan"*) kind=ft ;;
  *"isn't marked done"*) kind=kg ;;
  *"finished the task you delegated"*|*"tasks you delegated have ended"*) kind=res ;;
esac
if [ -n "$dir" ]; then
  mkdir -p "$dir/prompts"
  printf '%s' "$last" > "$dir/prompts/$kind-$(date +%s%N)-$$"
fi
out="ok"
if [ "$kind" = other ]; then
  hold=$(printf '%s\n' "$last" | sed -n 's/.*SPECHOLD:\([^ ]*\).*/\1/p' | head -n 1)
  if [ -n "$hold" ]; then
    i=0
    while [ ! -f "$hold" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
  fi
  if [ -n "$dir" ] && [ -s "$dir/other" ]; then
    out=$(head -n 1 "$dir/other")
    tail -n +2 "$dir/other" > "$dir/other.rest"
    mv "$dir/other.rest" "$dir/other"
  fi
elif [ -n "$dir" ] && [ -s "$dir/$kind" ]; then
  out=$(head -n 1 "$dir/$kind")
  tail -n +2 "$dir/$kind" > "$dir/$kind.rest"
  mv "$dir/$kind.rest" "$dir/$kind"
else
  out="Here is where things stand."
fi
case "$out" in
  FAILED:*)
    out=${out#FAILED:}
    printf '{"type":"assistant","message":{"id":"msg_f","content":[{"type":"tool_use","id":"toolu_f","name":"Bash","input":{"command":"run_tests.sh"}}]}}\n'
    printf '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_f","is_error":true,"content":"Permission to use Bash has been denied."}]}}\n'
    ;;
  TFAILED:*)
    out=${out#TFAILED:}
    printf '{"type":"assistant","message":{"id":"msg_t","content":[{"type":"tool_use","id":"toolu_t","name":"run_terminal_command","input":{"command":"journalctl -u maker --since 17:18"}}]}}\n'
    printf '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_t","is_error":true,"content":"Error: no terminal session is open."}]}}\n'
    ;;
  TOK:*)
    out=${out#TOK:}
    printf '{"type":"assistant","message":{"id":"msg_o","content":[{"type":"tool_use","id":"toolu_o","name":"run_terminal_command","input":{"command":"journalctl -u maker --since 17:18"}}]}}\n'
    printf '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_o","is_error":false,"content":"maker started, maker_first=true"}]}}\n'
    ;;
  HOLD:*)
    f=${out#HOLD:}; f=${f%% *}; out=${out#HOLD:* }
    i=0
    while [ ! -f "$f" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
    ;;
esac
printf '{"type":"result","is_error":false,"num_turns":1,"result":"%s"}\n' "$out"
"#;

fn fake_on_path() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    });
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
    lead: String,
    lead_name: String,
    checker: String,
    channel: String,
    tmp: tempfile::TempDir,
}

impl Env {
    fn script(&self) -> PathBuf {
        self.tmp.path().join("script")
    }

    fn file(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }

    /// What the lead answers on its results turns (`res`) and keep-going turns (`kg`), in order.
    fn queue(&self, kind: &str, lines: &[&str]) {
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(self.script().join(kind), text).unwrap();
    }

    /// The prompts the fake got of `kind`, oldest first.
    fn prompts(&self, kind: &str) -> Vec<String> {
        let Ok(dir) = std::fs::read_dir(self.script().join("prompts")) else { return Vec::new() };
        let mut files: Vec<PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{kind}-"))))
            .collect();
        files.sort();
        files.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect()
    }

    fn marker(&self) -> String {
        format!("SCRIPT:{} ", self.script().display())
    }
}

/// A team led by the first built-in bot, with a new teammate "Checker", in its own channel.
async fn setup() -> Env {
    fake_on_path();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let app = create_router(state.clone());
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants.as_array().unwrap()[0].clone();
    let lead = k["id"].as_str().unwrap().to_string();
    let lead_name = k["name"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{lead}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&app, "POST", "/api/kwants", Some(json!({"name": "Checker", "title": "Reviewer", "persona_prompt": "", "soul": "", "avatar_sprite": "default", "driver_type": "cli", "preferred_model": "claude"}))).await;
    assert!(s.is_success(), "{s} {v}");
    let checker = v["id"].as_str().or(v["kwant"]["id"].as_str()).unwrap().to_string();
    let (s, v) = call(&app, "POST", "/api/teams", Some(json!({"name": "keep", "lead_kwant_id": lead, "member_kwants": [lead, checker]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, channels) = call(&app, "GET", "/api/channels", None).await;
    let channel = channels.as_array().unwrap().iter().find(|c| c["name"] == "team-keep").unwrap()["id"].as_str().unwrap().to_string();
    let env = Env { app, state, lead, lead_name, checker, channel, tmp };
    std::fs::create_dir_all(env.script()).unwrap();
    env
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

/// What still runs for the test's chat: a turn, a job holding the chat, or one of the chat's
/// hand-offs that runs or whose job is held.
/// - A lead resumed with results holds the chat until its keep-going turns and the card they may
///   open are done: they run inside that job (delegation.rs `continue_lead`, then
///   `keep_going::after_lead_turn`). An answered card holds it too (questions.rs `resume`,
///   keep_going.rs `on_card_answer`), and so does the person's message (routes.rs `send_chat`).
/// - A hand-off runs in a job keyed by its own id (delegation.rs `spawn_run`). That job ends with
///   the specialist's turn, before `finish` saves the result and resumes the lead (`run` drops it
///   first), so a test whose chain still has a hand-off to finish waits for the lead's results
///   turn on it before it settles.
async fn running(env: &Env) -> Vec<String> {
    let turns = &env.state.turns;
    let mut what: Vec<String> = turns.active_turns().iter().map(|t| format!("{} turn of {} in {}", t.state, t.kwant_id, t.context_id)).collect();
    if turns.has_job(&env.channel) {
        what.push(format!("a job holding the chat: {:?}", turns.job_keys_holding(&env.channel)));
    }
    for d in delegations(env).await {
        if d.origin_channel_id.as_deref() == Some(env.channel.as_str()) && (d.status == "running" || turns.has_job(&d.id)) {
            what.push(format!("the hand-off \u{201c}{}\u{201d} ({}): {:?}", d.task, d.status, turns.job_keys_holding(&d.id)));
        }
    }
    what
}

/// Waits until the chat's work has ended, then checks nothing starts there again.
async fn settled(env: &Env) {
    wait_for("the chat's work to end", || async { running(env).await.is_empty() }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let late = running(env).await;
    assert!(late.is_empty(), "started in the chat after its work ended: {late:?}");
}

async fn messages(env: &Env) -> Vec<Message> {
    env.state.db.list_messages(&env.channel, 200).await.unwrap()
}

fn meta_of(m: &Message) -> Value {
    serde_json::from_str(m.meta.as_deref().unwrap_or("null")).unwrap_or(Value::Null)
}

fn lead_replies<'a>(env: &Env, msgs: &'a [Message]) -> Vec<&'a Message> {
    msgs.iter().filter(|m| m.sender_role == "kwant" && (m.sender_id == env.lead_name || m.sender_id == env.lead)).collect()
}

/// The person asks the lead in the team channel, and the lead's first turn answers.
async fn person_asks(env: &Env, text: &str) {
    let prompt = format!("@{} {text} {}please", env.lead_name, env.marker());
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"channel_id": env.channel, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("the lead's first answer", || async { messages(env).await.iter().any(|m| m.sender_role == "kwant" && m.text == "ok") }).await;
}

/// A hand-off from the lead to Checker from the team channel (resuming the lead when it ends,
/// unless `resumes` is false); returns its id.
async fn hand_off(env: &Env, task: &str, resumes: bool) -> String {
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({
            "lead_kwant_id": env.lead,
            "specialist_kwant_id": env.checker,
            "task": task,
            "title": format!("Check {}", env.marker()),
            "channel_id": env.channel,
            "continue_lead": resumes,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    d["id"].as_str().unwrap().to_string()
}

async fn delegations(env: &Env) -> Vec<kwnt::db::threads::Delegation> {
    env.state.db.list_delegations(Some(&env.lead), None, 200).await.unwrap()
}

async fn open_questions(env: &Env) -> Vec<kwnt::db::questions::Question> {
    env.state.db.list_questions(Some("open"), Some(&env.channel), None).await.unwrap()
}

async fn card_meta(env: &Env, q: &kwnt::db::questions::Question) -> Value {
    let m = env.state.db.get_message(q.message_id.as_deref().unwrap()).await.unwrap().unwrap();
    meta_of(&m)
}

async fn answer(env: &Env, q: &str, text: &str) {
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{q}/answer"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// The case from Oleks's video: the lead's results turn says "Passed …" and stops. kwnt gives it
/// a keep-going turn that quotes the person's message; its @mention goes out as a hand-off, and
/// the next results turn ends with `Done:`, which is stored and ends the chain.
#[tokio::test]
async fn a_stalled_lead_gets_a_keep_going_turn_and_its_mention_goes_out() {
    let env = setup().await;
    env.queue("res", &["Passed Checker's floor-dot task.", "Both parts look right.\\nDone: checked part 1 and part 2 against the brief."]);
    env.queue("kg", &["@Checker, please check part 2 KGMARK."]);
    person_asks(&env, "Build the level, both parts.").await;
    hand_off(&env, "Check part 1", true).await;
    wait_for("the second hand-off", || async { delegations(&env).await.len() == 2 }).await;
    wait_for("the lead's results turn on the second hand-off", || async { env.prompts("res").len() == 2 }).await;
    settled(&env).await;

    let kg = env.prompts("kg");
    assert_eq!(kg.len(), 1, "one keep-going turn");
    assert!(kg[0].contains(keep_going::PROMPT), "{}", kg[0]);
    assert!(kg[0].contains("> @"), "it quotes the person: {}", kg[0]);
    assert!(kg[0].contains("Build the level, both parts."), "{}", kg[0]);
    assert!(kg[0].contains("End every reply in one of four ways"), "{}", kg[0]);
    let second = delegations(&env).await.into_iter().find(|d| d.task.contains("KGMARK")).expect("the keep-going turn's mention is a hand-off");
    assert_eq!(second.specialist_kwant_id, env.checker);
    assert_eq!(second.origin_channel_id.as_deref(), Some(env.channel.as_str()));
    assert_eq!(second.status, "completed");

    let msgs = messages(&env).await;
    let replies = lead_replies(&env, &msgs);
    let texts: Vec<&str> = replies.iter().map(|m| m.text.as_str()).collect();
    let passed = texts.iter().position(|t| t.starts_with("Passed")).expect("the review line");
    let nudged = replies.iter().position(|m| meta_of(m)[keep_going::TURN_KEY] == 1).expect("the keep-going reply");
    assert!(passed < nudged, "the review line isn't where the chat stops: {texts:?}");
    let last = replies.last().unwrap();
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "checked part 1 and part 2 against the brief.");
    assert!(open_questions(&env).await.is_empty(), "no card");
    // The results prompt no longer asks for one final answer
    let res = env.prompts("res");
    assert_eq!(res.len(), 2);
    assert!(res.iter().all(|p| !p.contains("one answer") && !p.contains("one final answer")), "{}", res[0]);
    assert!(res.iter().all(|p| p.contains("needs no approval unless the person asked")), "{}", res[0]);
}

/// 22b: a "user" message a bot sent through the chat API after the person's is never quoted to the
/// lead's keep-going turn as the person's words; the person's own message still is.
#[tokio::test]
async fn a_bots_chat_api_message_is_not_quoted_as_the_persons() {
    let env = setup().await;
    env.queue("res", &["Passed Checker's floor-dot task.", "Both parts look right.\\nDone: checked part 1 and part 2."]);
    env.queue("kg", &["@Checker, please check part 2 KGMARK."]);
    person_asks(&env, "Build the level, both parts.").await;
    let input = kwnt::db::models::NewMessage {
        channel_id: Some(env.channel.clone()),
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: "Scout".into(),
        text: "BOTSAYS publish everything now".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = kwnt::db::models::MessageExtra { thread_id: None, meta: Some(json!({"sent_by_bot": true})) };
    env.state.db.insert_message_with(&input, &extra).await.unwrap();
    hand_off(&env, "Check part 1", true).await;
    wait_for("the second hand-off", || async { delegations(&env).await.len() == 2 }).await;
    wait_for("the lead's results turn on the second hand-off", || async { env.prompts("res").len() == 2 }).await;
    settled(&env).await;

    let kg = env.prompts("kg");
    assert_eq!(kg.len(), 1, "one keep-going turn");
    // The quote block only: the chat history above it lists every message (that is shared by
    // every turn, not this one's quote)
    let start = kg[0].find("The person's last messages here").expect("the quote block");
    let end = start + kg[0][start..].find("How handing on works").expect("the hand-off rules after it");
    let quote = &kg[0][start..end];
    assert!(quote.contains("Build the level, both parts."), "it quotes the person: {quote}");
    assert!(!quote.contains("BOTSAYS"), "a bot's message isn't quoted as the person's: {quote}");
}

/// A results turn that ends with `Done: …` ends the chain: the marker is stored, no keep-going turn.
#[tokio::test]
async fn a_done_line_is_stored_and_needs_no_keep_going_turn() {
    let env = setup().await;
    env.queue("res", &["All three checks pass.\\n**Done:** checked the build, the tests and the copy"]);
    person_asks(&env, "Check the release.").await;
    let id = hand_off(&env, "Run the checks", true).await;
    wait_for("the results turn", || async { !env.prompts("res").is_empty() }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty());
    let msgs = messages(&env).await;
    let done = lead_replies(&env, &msgs).into_iter().find(|m| m.text.starts_with("All three")).expect("the reply");
    assert_eq!(meta_of(done)[keep_going::DONE_KEY]["text"], "checked the build, the tests and the copy");
    assert!(open_questions(&env).await.is_empty());
    assert_eq!(delegations(&env).await.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec![id.as_str()]);
    // 33d: the save step leaves a lead with hand-offs to its chain: one Finished row, the results
    // reply, with its member
    let jobs = team_jobs(&env).await;
    assert_eq!(jobs.iter().map(|j| j["id"].as_str().unwrap()).collect::<Vec<_>>(), [done.id.as_str()], "{jobs:?}");
    assert_eq!(jobs[0]["members"].as_array().map(Vec::len), Some(1), "{jobs:?}");
}

/// The Inbox's Finished team jobs (`kind: team_job`).
async fn team_jobs(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "team_job").cloned().collect()
}

/// The person writes to the lead in its direct chat (no team there); waits for a reply to it.
async fn person_writes_direct(env: &Env, first_line: &str) {
    let replies = || async { env.state.db.list_messages(&env.lead, 200).await.unwrap().iter().filter(|m| m.sender_role == "kwant").count() };
    let before = replies().await;
    let prompt = format!("{first_line}\n{}", env.marker());
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"kwant_id": env.lead, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("the bot's reply", || async { replies().await > before }).await;
    wait_for("the turn to end", || async { env.state.turns.active_turns().is_empty() }).await;
}

/// 33d (Mira's UX pass at 0.9.38, fix #1): one bot doing one job in its own chat ends with
/// `Done: …`. The reply gets the Done mark at save, and the Inbox's "Done while you were away"
/// lists it, titled with the request it closed even when the person wrote again later that day,
/// with no members (nobody was handed work). A later plain reply adds nothing.
#[tokio::test]
async fn a_solo_bots_done_reply_is_marked_and_listed_with_its_own_request() {
    let env = setup().await;
    env.queue("other", &["Saved the plan to trip.md.\\nDone: 3-day plan saved", "Here is a packing list."]);
    person_writes_direct(&env, "Plan a 3-day trip to Lisbon").await;
    let msgs = env.state.db.list_messages(&env.lead, 200).await.unwrap();
    let done = msgs.iter().find(|m| m.sender_role == "kwant" && m.text.starts_with("Saved the plan")).expect("the Done reply").clone();
    assert_eq!(meta_of(&done)[keep_going::DONE_KEY]["text"], "3-day plan saved", "{:?}", done.meta);
    assert!(meta_of(&done)[keep_going::DONE_KEY]["at"].is_string());
    // The person writes again later the same day; that reply ends with no Done line
    tokio::time::sleep(Duration::from_millis(1100)).await;
    person_writes_direct(&env, "Thanks! And a packing list?").await;
    let msgs = env.state.db.list_messages(&env.lead, 200).await.unwrap();
    let plain = msgs.iter().find(|m| m.text == "Here is a packing list.").expect("the plain reply");
    assert!(meta_of(plain).get(keep_going::DONE_KEY).is_none(), "{:?}", plain.meta);

    let jobs = team_jobs(&env).await;
    assert_eq!(jobs.iter().map(|j| j["id"].as_str().unwrap()).collect::<Vec<_>>(), [done.id.as_str()], "{jobs:?}");
    let job = &jobs[0];
    assert_eq!(job["title"], "Plan a 3-day trip to Lisbon", "{job}");
    assert_eq!(job["done_line"], "3-day plan saved", "{job}");
    assert_eq!(job["group"], "finished", "{job}");
    assert_eq!(job["lead"]["kwant_id"], json!(env.lead), "{job}");
    // A solo job names no members, so it doesn't read "1 bot"
    assert!(job.get("members").is_none(), "{job}");
    assert_eq!(job["handoffs"], 0, "{job}");
}

/// 33d: a member's `Done:` inside its hand-off (depth 1) is no Finished row of its own, and a
/// lead's first reply without a Done line isn't either.
#[tokio::test]
async fn a_members_done_in_its_hand_off_and_a_plain_reply_add_no_finished_row() {
    let env = setup().await;
    person_asks(&env, "Check the build.").await;
    env.queue("other", &["Built it and ran the tests.\\nDone: build checked, 212 tests pass"]);
    let id = hand_off(&env, &format!("Run the build {}", env.marker()), false).await;
    wait_for("the hand-off to end", || async {
        delegations(&env).await.iter().any(|d| d.id == id && !d.is_open())
    })
    .await;
    settled(&env).await;
    let d = delegations(&env).await.into_iter().find(|d| d.id == id).unwrap();
    let thread = env.state.db.list_messages(&d.thread_id, 200).await.unwrap();
    let member = thread.iter().find(|m| m.text.starts_with("Built it")).unwrap_or_else(|| panic!("the member's reply: {thread:?}"));
    assert!(meta_of(member).get(keep_going::DONE_KEY).is_none(), "{:?}", member.meta);
    for m in messages(&env).await {
        assert!(meta_of(&m).get(keep_going::DONE_KEY).is_none(), "{m:?}");
    }
    assert!(team_jobs(&env).await.is_empty());
}

/// The lead asked with the question tool (an open card in the chat): it waits on the person, so
/// no keep-going turn and no second card.
#[tokio::test]
async fn asking_with_the_question_tool_needs_no_keep_going_turn() {
    let env = setup().await;
    let release = env.file("asked");
    std::fs::write(env.script().join("res"), format!("HOLD:{} I asked you which cover to use.\n", release.display())).unwrap();
    person_asks(&env, "Make the cover.").await;
    hand_off(&env, "Draft two covers", true).await;
    wait_for("the results turn", || async { !env.prompts("res").is_empty() }).await;
    // The lead's question card, as its ask tool leaves it, while the turn runs
    let place = kwnt::db::questions::QuestionPlace { kwant_id: env.lead.clone(), channel_id: Some(env.channel.clone()), direct_kwant_id: None, thread_id: None, turn_id: None };
    env.state.db.create_question(&place, "Cover A or cover B?", &["A".into(), "B".into()]).await.unwrap();
    std::fs::write(&release, "go").unwrap();
    settled(&env).await;
    assert!(env.prompts("kg").is_empty());
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1, "only the lead's own card: {open:?}");
    assert_eq!(open[0].question, "Cover A or cover B?");
}

/// A reply whose last line asks the person in prose (something only they can approve, so the
/// decide-first round doesn't take it) becomes a question card at once, with no keep-going turn. Its answer resumes the chain: the lead's turn on it stalls, so it gets a
/// keep-going turn, which ends with `Done:`.
#[tokio::test]
async fn a_prose_question_becomes_a_card_and_its_answer_keeps_the_chain_going() {
    let env = setup().await;
    env.queue("res", &["Cover A is ready.\\nShould I publish cover A on the store now?"]);
    env.queue("kg", &["Published cover A.\\nDone: published cover A and checked the store page."]);
    person_asks(&env, "Make the cover.").await;
    hand_off(&env, "Draft two covers", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty(), "a question waits on the person");
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].question, "Should I publish cover A on the store now?");
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::ASK_KEY], true);
    // One reminder, for the card (the reply's plain "waiting on a reply" entry is closed)
    let waiting = env.state.db.list_waiting(None).await.unwrap();
    let live: Vec<_> = waiting.iter().filter(|w| w.kwant_id == env.lead && w.status != "closed").collect();
    assert_eq!(live.len(), 1, "{waiting:?}");
    assert_eq!(live[0].question_id.as_deref(), Some(open[0].id.as_str()));

    answer(&env, &open[0].id, "Yes, publish it").await;
    wait_for("the keep-going turn after the answer", || async { !env.prompts("kg").is_empty() }).await;
    settled(&env).await;
    assert_eq!(env.prompts("kg").len(), 1);
    let msgs = messages(&env).await;
    let last = *lead_replies(&env, &msgs).last().unwrap();
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "published cover A and checked the store page.");
    assert!(open_questions(&env).await.is_empty());
}

/// 0.9.4: a lead whose last line is `Blocked: …` gets no keep-going turn: the line is stored as
/// `meta.blocked` and becomes a question card at once, whose answer resumes the chain.
#[tokio::test]
async fn a_blocked_line_becomes_a_card_not_a_keep_going_turn() {
    let env = setup().await;
    env.queue("res", &["The render was refused twice.\\nBlocked: terminal commands are off for this bot on the shared Mac. Allow Terminal for this team, then say continue."]);
    env.queue("kg", &["Rendered it.\\nDone: rendered the intro and checked its length."]);
    person_asks(&env, "Render the intro.").await;
    hand_off(&env, "Cut the intro", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty(), "blocked: asked, never pushed on");
    let msgs = messages(&env).await;
    let blocked = lead_replies(&env, &msgs).into_iter().find(|m| m.text.starts_with("The render was refused")).expect("the reply");
    let text = "terminal commands are off for this bot on the shared Mac. Allow Terminal for this team, then say continue.";
    assert_eq!(meta_of(blocked)[keep_going::BLOCKED_KEY]["text"], text);
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].question, format!("Blocked: {text}"));
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::ASK_KEY], true);
    // 33f: exactly one Needs-you item, the card's (the turn's own plain "Blocked:" entry is replaced)
    assert_eq!(live_waiting(&env, &env.lead).await.iter().map(|w| w.question_id.clone()).collect::<Vec<_>>(), [Some(open[0].id.clone())]);

    // The person unblocks it: the chain resumes
    answer(&env, &open[0].id, "Allowed. Continue").await;
    wait_for("the keep-going turn after the answer", || async { !env.prompts("kg").is_empty() }).await;
    settled(&env).await;
    let msgs = messages(&env).await;
    let last = *lead_replies(&env, &msgs).last().unwrap();
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "rendered the intro and checked its length.");
}

/// What still waits on the person from `kwant`.
async fn live_waiting(env: &Env, kwant: &str) -> Vec<kwnt::db::waiting::WaitingItem> {
    env.state.db.list_waiting(None).await.unwrap().into_iter().filter(|w| w.kwant_id == kwant && w.status != "closed").collect()
}

/// 33f: a member's `Blocked:` inside its hand-off (depth 1) goes to its lead in the result, not to
/// Needs you: the member opens no waiting item.
#[tokio::test]
async fn a_members_blocked_line_in_its_hand_off_opens_nothing_for_the_person() {
    let env = setup().await;
    person_asks(&env, "Back up the manuscript.").await;
    env.queue("other", &["Blocked: the backup drive isn't mounted. Plug it in and tell me to try again."]);
    let id = hand_off(&env, &format!("Back it up {}", env.marker()), false).await;
    wait_for("the hand-off to end", || async { delegations(&env).await.iter().any(|d| d.id == id && !d.is_open()) }).await;
    settled(&env).await;
    let d = delegations(&env).await.into_iter().find(|d| d.id == id).unwrap();
    let thread = env.state.db.list_messages(&d.thread_id, 200).await.unwrap();
    assert!(thread.iter().any(|m| m.text.starts_with("Blocked: the backup drive")), "the member's reply: {thread:?}");
    let live = live_waiting(&env, &env.checker).await;
    assert!(live.is_empty(), "{live:?}");
}

/// 33f: a reply whose `Blocked:` line hands on to a teammate (`@Checker …`) is the teammate's to
/// unblock: it opens nothing for the person.
#[tokio::test]
async fn a_blocked_line_that_hands_on_to_a_teammate_opens_nothing() {
    let env = setup().await;
    env.queue("other", &["Blocked: the backup drive isn't mounted. @Checker please mount it on the build box."]);
    env.queue("res", &["Checker mounted it and I backed it up.\\nDone: backed up the manuscript."]);
    let prompt = format!("@{} Back up the manuscript. {}please", env.lead_name, env.marker());
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"channel_id": env.channel, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("the mention's hand-off to end", || async {
        delegations(&env).await.iter().any(|d| d.specialist_kwant_id == env.checker && !d.is_open())
    })
    .await;
    wait_for("the lead's results turn", || async { !env.prompts("res").is_empty() }).await;
    settled(&env).await;
    assert!(lead_replies(&env, &messages(&env).await).iter().any(|m| m.text.starts_with("Blocked: the backup drive")));
    let live = live_waiting(&env, &env.lead).await;
    assert!(live.is_empty(), "{live:?}");
}

/// 13a: a lead whose last tool step failed and whose reply waits on the person in prose, with no
/// `Blocked:` line, is blocked too: its ending is stored as `meta.blocked` (from "failed_step") and
/// becomes a question card at once, never a keep-going turn.
#[tokio::test]
async fn a_failed_step_and_a_prose_ask_become_a_card_not_a_keep_going_turn() {
    let env = setup().await;
    env.queue("res", &["FAILED:The test run was refused, so I have no results.\\n\\nIf you approve the command, I'll run the tests and report back."]);
    person_asks(&env, "Run the tests.").await;
    hand_off(&env, "Check the test setup", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty(), "blocked: asked, never pushed on");
    let msgs = messages(&env).await;
    let reply = lead_replies(&env, &msgs).into_iter().find(|m| m.text.starts_with("The test run was refused")).expect("the reply");
    let meta = meta_of(reply);
    let steps = meta["activity"]["steps"].as_array().cloned().unwrap_or_default();
    assert!(steps.iter().any(|s| s["kind"] == "tool" && s["status"] == "failed"), "the failed step is in the activity: {meta}");
    let text = "If you approve the command, I'll run the tests and report back.";
    assert_eq!(meta[keep_going::BLOCKED_KEY], json!({"text": text, "from": "failed_step"}));
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].question, format!("Blocked: {text}"));
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::ASK_KEY], true);
}

/// 22j, Oleks's Quant chat (2026-11-15): the lead's terminal step fails on every try and each
/// reply ends "I'll read the local repo…". The results turn gets its one follow-through
/// continuation, which fails the same way: that is one lead reply (the turn and its
/// continuation), then one Blocked card naming the step and its error. No keep-going turn, no
/// "Keep going?" card.
#[tokio::test]
async fn a_step_that_fails_on_every_try_is_one_reply_and_one_blocked_card() {
    let env = setup().await;
    env.queue("res", &["TFAILED:The last checks never landed a merge SHA or the live maker-first flag. I'll read the local repo and the open terminal, then answer from what those show."]);
    env.queue("ft", &["TFAILED:The earlier checks never landed a merge SHA or the live maker-first flag. I'll read the local repo and the open terminal, then answer from what those show."]);
    env.queue("kg", &["TFAILED:SHOULD NOT RUN. I'll read the local repo and the open terminal."]);
    person_asks(&env, "Is the maker-first fix merged and live?").await;
    let before = lead_replies(&env, &messages(&env).await).len();
    hand_off(&env, "Check the maker logs", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;

    assert_eq!(env.prompts("res").len(), 1);
    let ft = env.prompts("ft");
    assert_eq!(ft.len(), 1, "one follow-through continuation");
    assert!(ft[0].contains("Your last step (run_terminal_command: Running `journalctl -u maker --since 17:18`)"), "{}", ft[0]);
    assert!(env.prompts("kg").is_empty(), "no keep-going turn");
    let msgs = messages(&env).await;
    let replies = lead_replies(&env, &msgs);
    // kwant's Blocked card is a message of the lead's too (`meta.kind`), not a reply
    let new: Vec<&&Message> = replies[before..].iter().filter(|m| meta_of(m).get("kind").is_none()).collect();
    assert_eq!(new.len(), 2, "the turn and its continuation: {:?}", new.iter().map(|m| &m.text).collect::<Vec<_>>());
    assert!(new[0].text.starts_with("The last checks"), "{}", new[0].text);
    assert!(new[1].text.starts_with("The earlier checks"), "{}", new[1].text);
    let lead = env.lead_name.clone();
    assert!(new[1].text.contains(&format!("_kwant: run_terminal_command failed again, and {lead} didn't try another way or say why._")), "{}", new[1].text);
    let text = format!("{lead} couldn't run `journalctl -u maker --since 17:18`: Error: no terminal session is open.");
    assert_eq!(meta_of(new[1])[keep_going::BLOCKED_KEY], json!({"text": text, "from": "follow_through"}));
    let cards = env.state.db.list_questions(None, Some(&env.channel), None).await.unwrap();
    assert_eq!(cards.len(), 1, "one card: {cards:?}");
    assert_eq!(cards[0].question, format!("Blocked: {text}"));
    let meta = card_meta(&env, &cards[0]).await;
    assert_eq!(meta[keep_going::ASK_KEY], true);
    assert!(meta.get(keep_going::CARD_KEY).is_none(), "not a Keep going? card: {meta}");
}

/// 22j: a stalled reply after a failed step gets a keep-going turn that is told what failed; when
/// that turn fails the same step and says the same thing again, the chain ends with one Blocked
/// card, not a second keep-going turn.
#[tokio::test]
async fn a_keep_going_turn_hears_what_failed_and_a_repeat_ends_in_a_blocked_card() {
    let env = setup().await;
    env.queue("res", &["TFAILED:The merge checks found nothing yet in the maker logs."]);
    env.queue("kg", &["TFAILED:The merge checks found nothing in the maker logs.", "SHOULD NOT RUN"]);
    person_asks(&env, "Is the maker-first fix merged and live?").await;
    hand_off(&env, "Check the maker logs", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;

    let kg = env.prompts("kg");
    assert_eq!(kg.len(), 1, "one keep-going turn, then blocked");
    assert!(kg[0].contains(keep_going::PROMPT), "{}", kg[0]);
    assert!(
        kg[0].contains("Your last step (run_terminal_command: Running `journalctl -u maker --since 17:18`) failed: Error: no terminal session is open. Don't run the same command again."),
        "{}",
        kg[0]
    );
    assert!(kg[0].contains("or end with `Blocked: <what>. <what would unblock it>`"), "{}", kg[0]);
    let lead = env.lead_name.clone();
    let text = format!("{lead} couldn't run `journalctl -u maker --since 17:18`: Error: no terminal session is open.");
    let msgs = messages(&env).await;
    // the newest reply, not kwant's Blocked card after it (`meta.kind`)
    let last = *lead_replies(&env, &msgs).iter().rev().find(|m| meta_of(m).get("kind").is_none()).unwrap();
    assert_eq!(meta_of(last)[keep_going::TURN_KEY], 1);
    assert_eq!(meta_of(last)[keep_going::BLOCKED_KEY], json!({"text": text, "from": "repeat"}));
    let cards = env.state.db.list_questions(None, Some(&env.channel), None).await.unwrap();
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0].question, format!("Blocked: {text}"));
    assert_eq!(card_meta(&env, &cards[0]).await[keep_going::ASK_KEY], true);
}

/// 22j: when the turns after a failed step say something new each time but never finish, the
/// "Keep going?" card says what failed instead of quoting the plan.
#[tokio::test]
async fn the_keep_going_card_after_a_failed_step_says_what_failed() {
    let env = setup().await;
    env.queue("res", &["TFAILED:The merge checks found nothing yet."]);
    env.queue("kg", &["TFAILED:Tried the deploy log next, also empty.", "TFAILED:Looked at the release notes, no mention."]);
    person_asks(&env, "Is the maker-first fix merged and live?").await;
    hand_off(&env, "Check the maker logs", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    assert_eq!(env.prompts("kg").len(), 2);
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1);
    let lead = env.lead_name.clone();
    assert_eq!(open[0].question, format!("{lead} is stuck: `journalctl -u maker --since 17:18` failed (Error: no terminal session is open). Keep going?"));
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::CARD_KEY], true);
}

/// 22j round 2: Oleks pressed Keep going and the same loop started again. The first turn after
/// "Keep going" fails the same step and opens like the reply before the card: it ends Blocked at
/// once, with no further keep-going turn.
#[tokio::test]
async fn keep_going_then_the_same_reply_after_the_same_failed_step_ends_blocked() {
    let env = setup().await;
    env.queue("res", &["TFAILED:The merge checks found nothing yet."]);
    env.queue(
        "kg",
        &[
            "TFAILED:Tried the deploy log next, also empty.",
            "TFAILED:Looked at the release notes, no mention.",
            "TFAILED:Looked at the release notes again, no mention.",
            "SHOULD NOT RUN",
        ],
    );
    person_asks(&env, "Is the maker-first fix merged and live?").await;
    hand_off(&env, "Check the maker logs", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    let card = open_questions(&env).await.remove(0);
    assert_eq!(card_meta(&env, &card).await[keep_going::CARD_KEY], true, "the Keep going? card first");
    assert_eq!(env.prompts("kg").len(), 2);

    answer(&env, &card.id, keep_going::KEEP_GOING).await;
    wait_for("the Blocked card", || async { open_questions(&env).await.iter().any(|q| q.question.starts_with("Blocked: ")) }).await;
    settled(&env).await;

    let kg = env.prompts("kg");
    assert_eq!(kg.len(), 3, "one turn after Keep going, then blocked");
    assert!(kg.iter().any(|p| p.contains("Your last step (run_terminal_command: Running `journalctl -u maker --since 17:18`) failed")), "{kg:?}");
    let lead = env.lead_name.clone();
    let text = format!("{lead} couldn't run `journalctl -u maker --since 17:18`: Error: no terminal session is open.");
    let msgs = messages(&env).await;
    let last = *lead_replies(&env, &msgs).iter().rev().find(|m| meta_of(m).get("kind").is_none()).unwrap();
    assert!(last.text.starts_with("Looked at the release notes again"), "{}", last.text);
    assert_eq!(meta_of(last)[keep_going::TURN_KEY], 1, "the first turn of the fresh allowance");
    assert_eq!(meta_of(last)[keep_going::BLOCKED_KEY], json!({"text": text, "from": "repeat"}));
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].question, format!("Blocked: {text}"));
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::ASK_KEY], true);
    let cards = env.state.db.list_questions(None, Some(&env.channel), None).await.unwrap();
    assert_eq!(cards.len(), 2, "the answered Keep going? card and the Blocked card: {cards:?}");
}

/// 22j round 2: after "Keep going" the step works. The reply may open like the one before the
/// card, but it isn't blocked: the chain goes on.
#[tokio::test]
async fn keep_going_then_the_step_works_is_never_blocked() {
    let env = setup().await;
    env.queue("res", &["TFAILED:The merge checks found nothing yet."]);
    env.queue(
        "kg",
        &[
            "TFAILED:Tried the deploy log next, also empty.",
            "TFAILED:Looked at the release notes, no mention.",
            "TOK:Looked at the release notes again, no mention.",
            "Found the startup line.\\nDone: maker_first is on since the 17:18 restart.",
        ],
    );
    person_asks(&env, "Is the maker-first fix merged and live?").await;
    hand_off(&env, "Check the maker logs", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    let card = open_questions(&env).await.remove(0);
    assert_eq!(card_meta(&env, &card).await[keep_going::CARD_KEY], true);

    answer(&env, &card.id, keep_going::KEEP_GOING).await;
    wait_for("both turns after Keep going", || async { env.prompts("kg").len() == 4 }).await;
    settled(&env).await;

    let kg = env.prompts("kg");
    assert_eq!(kg.len(), 4, "the turn whose step worked goes on to another");
    assert_eq!(kg.iter().filter(|p| p.contains("Your last step (")).count(), 3, "the turn after the step worked hears no failure: {kg:?}");
    assert!(!kg[3].contains("Your last step ("), "{}", kg[3]);
    let cards = env.state.db.list_questions(None, Some(&env.channel), None).await.unwrap();
    assert!(cards.iter().all(|q| !q.question.starts_with("Blocked: ")), "{cards:?}");
    assert!(open_questions(&env).await.is_empty());
    let msgs = messages(&env).await;
    let replies = lead_replies(&env, &msgs);
    assert!(replies.iter().all(|m| meta_of(m).get(keep_going::BLOCKED_KEY).is_none()), "nothing blocked");
    let last = *replies.iter().rev().find(|m| meta_of(m).get("kind").is_none()).unwrap();
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "maker_first is on since the 17:18 restart.");
}

/// Two keep-going turns that still hand nothing on: one "Keep going?" card, never a second.
/// "Keep going" is a person message: a fresh allowance and a keep-going turn at once.
#[tokio::test]
async fn two_silent_keep_going_turns_end_in_one_card_and_keep_going_starts_another() {
    let env = setup().await;
    env.queue("res", &["Passed Checker's draft."]);
    env.queue("kg", &["Still looking at the draft.", "Reading it once more.", "Finished the last pass.\\nDone: checked the whole draft."]);
    person_asks(&env, "Write the chapter.").await;
    hand_off(&env, "Draft chapter 1", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    assert_eq!(env.prompts("kg").len(), 2, "the allowance is 2 turns");
    let open = open_questions(&env).await;
    assert_eq!(open.len(), 1);
    let lead = env.lead_name.clone();
    assert_eq!(open[0].question, format!("{lead} stopped before the job was done: Reading it once more. Keep going?"));
    assert_eq!(open[0].choice_list(), vec![keep_going::KEEP_GOING.to_string(), keep_going::ENOUGH.to_string()]);
    assert_eq!(card_meta(&env, &open[0]).await[keep_going::CARD_KEY], true);
    let msgs = messages(&env).await;
    let turns: Vec<Value> = lead_replies(&env, &msgs).iter().map(|m| meta_of(m)[keep_going::TURN_KEY].clone()).filter(|v| !v.is_null()).collect();
    assert_eq!(turns, vec![json!(1), json!(2)]);

    answer(&env, &open[0].id, keep_going::KEEP_GOING).await;
    wait_for("the third keep-going turn", || async { env.prompts("kg").len() == 3 }).await;
    settled(&env).await;
    assert_eq!(env.prompts("kg").len(), 3);
    let msgs = messages(&env).await;
    let last = *lead_replies(&env, &msgs).last().unwrap();
    assert_eq!(meta_of(last)[keep_going::TURN_KEY], 1, "a fresh allowance");
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "checked the whole draft.");
    let cards = env.state.db.list_questions(None, Some(&env.channel), None).await.unwrap();
    assert_eq!(cards.len(), 1, "never a second card: {cards:?}");
}

/// "That's enough for now" ends the chain: no keep-going turn.
#[tokio::test]
async fn thats_enough_for_now_ends_the_chain() {
    let env = setup().await;
    env.queue("res", &["Passed Checker's draft."]);
    env.queue("kg", &["Still looking.", "Still looking.", "SHOULD NOT RUN"]);
    person_asks(&env, "Write the chapter.").await;
    hand_off(&env, "Draft chapter 1", true).await;
    wait_for("the card", || async { !open_questions(&env).await.is_empty() }).await;
    settled(&env).await;
    let q = open_questions(&env).await.remove(0);
    answer(&env, &q.id, keep_going::ENOUGH).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    settled(&env).await;
    assert_eq!(env.prompts("kg").len(), 2);
    assert!(open_questions(&env).await.is_empty());
}

/// The person writes while the lead's results turn runs: their message has its own turn, so the
/// chain isn't nudged.
#[tokio::test]
async fn the_person_writing_means_no_keep_going_turn() {
    let env = setup().await;
    let release = env.file("person-wrote");
    std::fs::write(env.script().join("res"), format!("HOLD:{} Passed Checker's draft.\n", release.display())).unwrap();
    person_asks(&env, "Write the chapter.").await;
    hand_off(&env, "Draft chapter 1", true).await;
    wait_for("the results turn", || async { !env.prompts("res").is_empty() }).await;
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"channel_id": env.channel, "prompt": "Actually, I'll read it myself.", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    std::fs::write(&release, "go").unwrap();
    wait_for("the results reply", || async { messages(&env).await.iter().any(|m| m.text == "Passed Checker's draft.") }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty());
    assert!(open_questions(&env).await.is_empty());
}

/// At the hand-off ceiling the lead's mention waits on Continue: that's a pause the person lifts,
/// not a stall.
#[tokio::test]
async fn the_hand_off_ceiling_means_no_keep_going_turn() {
    let env = setup().await;
    env.queue("res", &["@Checker, please check part 3."]);
    person_asks(&env, "Check every part.").await;
    // This request already started the most hand-offs it may (all ended)
    for i in 0..kwnt::server::routes::MAX_DELEGATIONS_PER_REQUEST {
        let (_, d) = env
            .state
            .db
            .create_delegation(&kwnt::db::threads::NewDelegation {
                lead_kwant_id: env.lead.clone(),
                specialist_kwant_id: env.checker.clone(),
                origin_channel_id: Some(env.channel.clone()),
                origin_kwant_id: None,
                origin_thread_id: None,
                team_id: None,
                task: format!("earlier round {i}"),
                title: "Earlier round".into(),
                continue_lead: true,
                depth: 1,
                follow_up_at: None,
            })
            .await
            .unwrap();
        assert!(env.state.db.finish_delegation(&d.id, "completed", None, None).await.unwrap());
    }
    hand_off(&env, "Check part 2", true).await;
    wait_for("the Continue notice", || async {
        messages(&env).await.iter().any(|m| meta_of(m)["kind"] == kwnt::server::budget::CAPPED_KIND)
    })
    .await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty());
    assert!(open_questions(&env).await.is_empty());
}

/// A chain budget pause posted during the results turn holds the chain until the person lifts it.
#[tokio::test]
async fn a_budget_pause_means_no_keep_going_turn() {
    let env = setup().await;
    let release = env.file("paused");
    std::fs::write(env.script().join("res"), format!("HOLD:{} Passed Checker's draft.\n", release.display())).unwrap();
    person_asks(&env, "Write the chapter.").await;
    hand_off(&env, "Draft chapter 1", true).await;
    wait_for("the results turn", || async { !env.prompts("res").is_empty() }).await;
    env.state
        .db
        .insert_message_with(
            &kwnt::db::models::NewMessage {
                channel_id: Some(env.channel.clone()),
                direct_kwant_id: None,
                sender_role: "system".into(),
                sender_id: "kwnt".into(),
                text: "Paused: this chain used its budget.".into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            },
            &kwnt::db::models::MessageExtra { thread_id: None, meta: Some(json!({"kind": kwnt::server::budget::KIND})) },
        )
        .await
        .unwrap();
    std::fs::write(&release, "go").unwrap();
    wait_for("the results reply", || async { messages(&env).await.iter().any(|m| m.text == "Passed Checker's draft.") }).await;
    settled(&env).await;
    assert!(env.prompts("kg").is_empty());
    assert!(open_questions(&env).await.is_empty());
}

/// Another hand-off of the lead from this chat is still running: the chain is moving.
#[tokio::test]
async fn another_open_hand_off_means_no_keep_going_turn() {
    let env = setup().await;
    env.queue("res", &["Passed Checker's part 1."]);
    person_asks(&env, "Check both parts.").await;
    let release = env.file("part2");
    let running = hand_off(&env, &format!("Check part 2 SPECHOLD:{}", release.display()), false).await;
    wait_for("part 2 to run", || async {
        env.state.db.get_delegation(&running).await.unwrap().is_some_and(|d| d.status == "running")
    })
    .await;
    hand_off(&env, "Check part 1", true).await;
    wait_for("the results turn", || async { !env.prompts("res").is_empty() }).await;
    wait_for("the results reply", || async { messages(&env).await.iter().any(|m| m.text == "Passed Checker's part 1.") }).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(env.prompts("kg").is_empty(), "part 2 still runs");
    assert!(open_questions(&env).await.is_empty());
    std::fs::write(&release, "go").unwrap();
    settled(&env).await;
}

/// The lead's prompts: no "one answer" left, and the hand-off rules name the four ends.
#[test]
fn the_prompts_name_the_three_ends() {
    let rules = delegation::handoff_rules();
    assert!(!rules.contains("one answer"), "{rules}");
    assert!(rules.contains("End every reply in one of four ways"), "{rules}");
    assert!(rules.contains("@mention, or do it yourself in this reply"), "{rules}");
    assert!(rules.contains("ask them with the question tool"), "{rules}");
    assert!(rules.contains("`Done: <what you checked>`"), "{rules}");
    assert!(rules.contains("`Blocked: <what is blocked>. <what would unblock it>`"), "{rules}");
    assert!(keep_going::PROMPT.starts_with("The person's request isn't marked done"));
}
