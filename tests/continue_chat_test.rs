//! "Continue in a new chat from here": `POST /api/messages/:id/continue` (brief:
//! shared/ship-software-safely/autonomy/continue-chat/brief.md). A new chat with the same bot
//! holds a copy of the old one up to and including the pressed message; the old one doesn't
//! change; the new chat's next turn reads the copies; only the person can do it.

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
    db::{
        attachments::NewAttachment,
        models::{MessageExtra, NewChannel, NewMessage},
        spaces::{NewSpace, SpaceId},
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// Records each prompt and answers "answer".
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$$-$(date +%s%N)"
echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer"}'
"#;

fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        prompts
    })
}

fn prompts_with(marker: &str) -> Vec<String> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| p.contains(marker))
        .collect()
}

struct Env {
    app: Router,
    state: AppState,
    bot: String,
    other: String,
    /// Bot replies are stored under the bot's name (what the prompt builder matches on)
    bot_name: String,
    _tmp: tempfile::TempDir,
}

async fn call(app: &Router, method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn setup() -> Env {
    prompts_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
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
    let (_, kwants) = call(&app, "GET", "/api/kwants", None, None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(2) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), None, Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    let bot_name = state.db.get_kwant(&ids[0]).await.unwrap().unwrap().name;
    Env { app, state, bot: ids[0].clone(), other: ids[1].clone(), bot_name, _tmp: tmp }
}

impl Env {
    /// A message in a bot's main chat (`direct`), a thread or a channel.
    async fn say(&self, direct: Option<&str>, thread: Option<&str>, channel: Option<&str>, role: &str, text: &str) -> String {
        let msg = NewMessage {
            channel_id: channel.map(Into::into),
            direct_kwant_id: direct.map(Into::into),
            sender_role: role.into(),
            sender_id: if role == "user" { "user".into() } else { self.bot_name.clone() },
            text: text.into(),
            reasoning_content: Some(format!("why {text}")),
            tool_calls: Some(json!([{"name": "read", "output": text}])),
            tokens_used: 7,
        };
        let extra = MessageExtra { thread_id: thread.map(Into::into), meta: None };
        self.state.db.insert_message_with(&msg, &extra).await.unwrap().id
    }

    async fn texts(&self, context: &str) -> Vec<String> {
        self.state.db.list_messages(context, 2000).await.unwrap().into_iter().map(|m| m.text).collect()
    }

    async fn continue_from(&self, id: &str) -> (StatusCode, Value) {
        call(&self.app, "POST", &format!("/api/messages/{id}/continue"), None, None).await
    }
}

#[tokio::test]
async fn continuing_the_main_chat_copies_up_to_the_pressed_message_and_leaves_it_as_it_was() {
    let e = setup().await;
    let k = e.bot.clone();
    e.say(Some(&k), None, None, "user", "first").await;
    e.say(Some(&k), None, None, "kwant", "second").await;
    let cut = e.say(Some(&k), None, None, "user", "third").await;
    e.say(Some(&k), None, None, "kwant", "fourth, after").await;
    let before = e.state.db.list_messages(&k, 100).await.unwrap();

    let (s, v) = e.continue_from(&cut).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["copied"].as_u64(), v["truncated"].as_bool()), (Some(3), Some(false)));
    assert_eq!(v["kind"], "chat");
    assert_eq!(v["kwant_id"], k.as_str());
    assert_eq!(v["created_by"], "user");
    assert_eq!(v["forked_from_context"], k.as_str());
    assert_eq!(v["forked_from_message_id"], cut.as_str());
    let name = e.state.db.get_kwant(&k).await.unwrap().unwrap().name;
    assert_eq!(v["title"], format!("{name} (continued)"));
    let tid = v["id"].as_str().unwrap().to_string();

    assert_eq!(e.texts(&tid).await, ["first", "second", "third"]);
    let copies = e.state.db.list_messages(&tid, 100).await.unwrap();
    for (c, o) in copies.iter().zip(&before) {
        assert_ne!(c.id, o.id);
        assert_eq!(
            (&c.sender_role, &c.sender_id, &c.reasoning_content, &c.tool_calls, c.tokens_used, c.created_at),
            (&o.sender_role, &o.sender_id, &o.reasoning_content, &o.tool_calls, o.tokens_used, o.created_at)
        );
    }
    // The original chat is unchanged
    let after = e.state.db.list_messages(&k, 100).await.unwrap();
    let ids = |v: &[kwnt::db::models::Message]| v.iter().map(|m| (m.id.clone(), m.text.clone(), m.thread_id.clone())).collect::<Vec<_>>();
    assert_eq!(ids(&after), ids(&before));
    // GET shows the same thread with the two new fields; the bot's thread list has it
    let (s, t) = call(&e.app, "GET", &format!("/api/threads/{tid}"), None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["forked_from_message_id"], cut.as_str());
    assert_eq!((t["forked_copied"].as_i64(), t["forked_truncated"].as_bool()), (Some(3), Some(false)), "kept for a reload");
    let (_, list) = call(&e.app, "GET", &format!("/api/kwants/{k}/threads"), None, None).await;
    assert!(list.as_array().unwrap().iter().any(|t| t["id"] == tid.as_str()));
    // A plain new thread has neither
    let (_, plain) = call(&e.app, "POST", &format!("/api/kwants/{k}/threads"), None, Some(json!({"title": "x"}))).await;
    for f in ["forked_from_context", "forked_from_message_id", "forked_copied", "forked_truncated"] {
        assert!(plain[f].is_null(), "{f}: {plain}");
    }
}

#[tokio::test]
async fn continuing_a_thread_keeps_its_order_and_the_next_turn_reads_the_copies() {
    let e = setup().await;
    let k = e.bot.clone();
    let t = e.state.db.create_thread(&k, "Wifi help", "chat", None, "user").await.unwrap();
    for i in 0..25 {
        e.say(None, Some(&t.id), None, if i % 2 == 0 { "user" } else { "kwant" }, &format!("OLDMARK{i:02}")).await;
    }
    let cut = e.state.db.list_messages(&t.id, 100).await.unwrap()[22].id.clone();
    let before = e.texts(&t.id).await;

    let (s, v) = e.continue_from(&cut).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["copied"], 23);
    assert_eq!(v["title"], "Wifi help (continued)");
    assert_eq!(v["forked_from_context"], t.id.as_str());
    let tid = v["id"].as_str().unwrap().to_string();
    let want: Vec<String> = (0..23).map(|i| format!("OLDMARK{i:02}")).collect();
    assert_eq!(e.texts(&tid).await, want, "oldest first, up to and including the pressed one");
    assert_eq!(e.texts(&t.id).await, before, "the source thread is unchanged");

    // The new chat's next turn reads the copies as its history (routes.rs loads 20 rows; the CLI
    // prompt shows the newest 10), the bot's copied replies as its own
    let (s, r) = call(&e.app, "POST", "/api/chat/send", None, Some(json!({"thread_id": tid, "prompt": "NEXTTURN go on", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    for _ in 0..300 {
        if e.texts(&tid).await.iter().any(|m| m == "answer") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let prompts = prompts_with("NEXTTURN");
    assert_eq!(prompts.len(), 1, "one turn");
    let p = &prompts[0];
    assert!(p.contains("User: OLDMARK22") && p.contains("Assistant: OLDMARK21") && p.contains("OLDMARK13"), "the copies are its history: {p}");
    assert!(!p.contains("another assistant"), "its own copied replies aren't a teammate's: {p}");
    assert!(!p.contains("OLDMARK23") && !p.contains("OLDMARK24"), "nothing after the pressed message");
    assert!(e.texts(&t.id).await.iter().all(|m| m != "answer" && !m.contains("NEXTTURN")), "the old thread got nothing");
}

#[tokio::test]
async fn only_a_chat_with_one_bot_can_be_continued() {
    let e = setup().await;
    let space = SpaceId::default_space();
    let channel = e
        .state
        .db
        .create_channel(
            &space,
            &NewChannel {
                name: "team-room".into(),
                description: None,
                lead_kwant_id: e.bot.clone(),
                member_kwants: vec![e.bot.clone(), e.other.clone()],
                routing_mode: "auto".into(),
            },
        )
        .await
        .unwrap();
    let in_channel = e.say(None, None, Some(&channel.id), "user", "hello team").await;
    let (s, v) = e.continue_from(&in_channel).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "Only a chat with one bot can be continued.");

    let handoff = e.state.db.create_thread(&e.other, "A task", "delegation", None, &e.bot).await.unwrap();
    let in_handoff = e.say(None, Some(&handoff.id), None, "user", "do it").await;
    let (s, v) = e.continue_from(&in_handoff).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // A context that is no chat at all (a stray direct id)
    let stray = e.say(Some("not-a-bot"), None, None, "user", "?").await;
    assert_eq!(e.continue_from(&stray).await.0, StatusCode::BAD_REQUEST);

    let (s, v) = e.continue_from("no-such-message").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string(), "{v}");

    for k in [&e.bot, &e.other] {
        let (_, list) = call(&e.app, "GET", &format!("/api/kwants/{k}/threads"), None, None).await;
        assert!(list.as_array().unwrap().iter().all(|t| t["forked_from_context"].is_null()), "nothing was made");
    }
}

#[tokio::test]
async fn bot_tokens_get_403_and_another_space_404() {
    let e = setup().await;
    let msg = e.say(Some(&e.bot), None, None, "user", "hi").await;
    let bot = e.state.db.get_kwant(&e.bot).await.unwrap().unwrap();
    let folder = e._tmp.path().join("ws");
    std::fs::create_dir_all(&folder).unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, token) = mcp_runtime::prepare_turn_with_self(&e.state, &bot, None, Some(folder), Some(scope)).await.unwrap();
    let uri = format!("/api/messages/{msg}/continue");
    let (s, v) = call(&e.app, "POST", &uri, Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, _) = call(&e.app, "POST", &uri, Some("wrong-token"), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let person = e.state.api_token.reveal().to_string();

    let other = e
        .state
        .db
        .create_space(&NewSpace { name: "Side".into(), icon: "S".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    let (s, _) = call(&e.app, "POST", &format!("{uri}?space={}", other.id), Some(&person), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a message from another space");
    let (_, list) = call(&e.app, "GET", &format!("/api/kwants/{}/threads", e.bot), None, None).await;
    assert!(list.as_array().unwrap().is_empty(), "nothing was made");

    let (s, v) = call(&e.app, "POST", &format!("{uri}?space={}", SpaceId::default_space().as_str()), Some(&person), None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[tokio::test]
async fn over_1000_messages_copies_the_newest_1000() {
    let e = setup().await;
    let k = e.bot.clone();
    for i in 0..1003 {
        e.say(Some(&k), None, None, "user", &format!("m{i:04}")).await;
    }
    let cut = e.state.db.list_messages(&k, 2000).await.unwrap()[1001].id.clone();
    let (s, v) = e.continue_from(&cut).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["copied"].as_u64(), v["truncated"].as_bool()), (Some(1000), Some(true)));
    assert_eq!((v["forked_copied"].as_i64(), v["forked_truncated"].as_bool()), (Some(1000), Some(true)));
    let got = e.texts(v["id"].as_str().unwrap()).await;
    assert_eq!(got.len(), 1000);
    assert_eq!((got[0].as_str(), got[999].as_str()), ("m0002", "m1001"));

    // Exactly 1,000 up to the cut is not truncated
    let cut = e.state.db.list_messages(&k, 2000).await.unwrap()[999].id.clone();
    let (_, v) = e.continue_from(&cut).await;
    assert_eq!((v["copied"].as_u64(), v["truncated"].as_bool()), (Some(1000), Some(false)));
}

#[tokio::test]
async fn files_come_along_by_reference_for_the_bot_up_to_the_cut() {
    let e = setup().await;
    let k = e.bot.clone();
    let mut files = Vec::new();
    let mut said = Vec::new();
    for name in ["before.csv", "after.csv"] {
        let a = e
            .state
            .db
            .insert_attachment(&NewAttachment {
                sha256: "0".repeat(64),
                name: name.into(),
                mime: "text/csv".into(),
                size: 4,
                source: "user".into(),
                context_id: Some(k.clone()),
                direct_kwant_id: Some(k.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let m = e.say(Some(&k), None, None, "user", &format!("here is {name}")).await;
        e.state.db.link_attachments(std::slice::from_ref(&a.id), &m, &k, None, Some(&k), None).await.unwrap();
        files.push(a);
        said.push(m);
    }
    let (s, v) = e.continue_from(&said[0]).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let tid = v["id"].as_str().unwrap().to_string();
    let bot = e.state.db.get_kwant(&k).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { thread_id: Some(tid.clone()), ..Default::default() }),
        ..Default::default()
    };
    let (list, err) = kwnt::server::attachments::tool_list(&e.state, &scope).await;
    assert!(!err, "{list}");
    assert!(list.contains("before.csv") && !list.contains("after.csv"), "{list}");
    // A thread that wasn't continued sees neither
    let plain = e.state.db.create_thread(&k, "x", "chat", None, "user").await.unwrap();
    let scope = SelfScope { origin: Some(TurnOrigin { thread_id: Some(plain.id), ..Default::default() }), ..scope };
    let (list, _) = kwnt::server::attachments::tool_list(&e.state, &scope).await;
    assert!(!list.contains("before.csv"), "{list}");
}
