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
/// answer), which replies "ok"; a teammate's task with "SPECHOLD:<file>" waits for that file
/// first. Each queue line is one reply (`\n` for a line break); an empty queue replies "Here is
/// where things stand.". A line "HOLD:<file> <reply>" waits for the file, then replies. Every
/// prompt is saved as `<dir>/prompts/<kind>-…`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
dir=$(printf '%s\n' "$last" | sed -n 's/.*SCRIPT:\([^ ]*\).*/\1/p' | head -n 1)
kind=other
case "$last" in
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
elif [ -n "$dir" ] && [ -s "$dir/$kind" ]; then
  out=$(head -n 1 "$dir/$kind")
  tail -n +2 "$dir/$kind" > "$dir/$kind.rest"
  mv "$dir/$kind.rest" "$dir/$kind"
else
  out="Here is where things stand."
fi
case "$out" in
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

/// No turn has run for a second: the chain has settled.
async fn settled(env: &Env) {
    let mut quiet = 0;
    for _ in 0..300 {
        let (_, t) = call(&env.app, "GET", "/api/turns", None).await;
        let busy = t["turns"].as_array().is_some_and(|a| !a.is_empty());
        quiet = if busy { 0 } else { quiet + 1 };
        if quiet >= 5 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("turns kept running");
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

    // The person unblocks it: the chain resumes
    answer(&env, &open[0].id, "Allowed. Continue").await;
    wait_for("the keep-going turn after the answer", || async { !env.prompts("kg").is_empty() }).await;
    settled(&env).await;
    let msgs = messages(&env).await;
    let last = *lead_replies(&env, &msgs).last().unwrap();
    assert_eq!(meta_of(last)[keep_going::DONE_KEY]["text"], "rendered the intro and checked its length.");
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
