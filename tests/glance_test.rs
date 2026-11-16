//! The desktop island's glance (0.9.13): `GET /api/glance` and `POST /api/glance/ask`, and the
//! approval and question calls the desktop app makes with the person's cookie.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::{ApprovalDecision, ApprovalRequest, PermissionBroker},
    config::Config,
    db::{
        models::{NewApproval, NewChannel, NewKwant, NewMessage, UpdateKwant},
        questions::QuestionPlace,
        spaces::{NewSpace, SpaceId},
        threads::NewDelegation,
        Database,
    },
    drivers::events::{RuntimeEvent, TurnTag},
    router::LayaRouter,
    server::{
        activity::TurnActivity,
        auth::{ApiToken, Require},
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "glance-test-token-0123456789abcdef";
const REMOTE: &str = "203.0.113.9:40000";
const SECRET: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let data = tmp.path().to_path_buf();
    let db = Database::init(&data).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    db.set_space_approvals(&SpaceId::default_space(), "ask").await.unwrap();
    let config = Config { data_dir: data.clone(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    (create_router(state.clone()), state, tmp)
}

#[derive(Default)]
struct Req<'a> {
    body: Option<Value>,
    /// From another machine (the desktop app reaching a server), else from loopback.
    remote: bool,
    bearer: Option<&'a str>,
    cookie: Option<&'a str>,
    origin: Option<&'a str>,
    if_none_match: Option<&'a str>,
}

async fn send(app: &Router, method: &str, uri: &str, r: Req<'_>) -> (StatusCode, HeaderMap, Value) {
    let host = if r.remote { "kwnt.example.com" } else { "127.0.0.1:8799" };
    let mut b = Request::builder().method(method).uri(uri).header("host", host);
    if let Some(t) = r.bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if let Some(c) = r.cookie {
        b = b.header("cookie", format!("kwnt_token={c}"));
    }
    if let Some(o) = r.origin {
        b = b.header("origin", o);
    }
    if let Some(e) = r.if_none_match {
        b = b.header("if-none-match", e);
    }
    let body = match r.body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let mut req = b.body(body).unwrap();
    if r.remote {
        req.extensions_mut().insert(ConnectInfo(REMOTE.parse::<SocketAddr>().unwrap()));
    }
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = to_bytes(res.into_body(), 1 << 22).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn glance(app: &Router) -> (Value, String) {
    let (s, h, v) = send(app, "GET", "/api/glance", Req::default()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (v, h["etag"].to_str().unwrap().to_string())
}

async fn approval(state: &AppState, kwant: &str, tool_name: &str, details: Value) -> (String, tokio::sync::oneshot::Receiver<ApprovalDecision>) {
    let a = state
        .db
        .save_approval(&NewApproval { turn_id: "turn-gone".into(), kwant_id: kwant.into(), tool_name: tool_name.into(), parameters: details })
        .await
        .unwrap();
    let rx = state
        .broker
        .request_approval(ApprovalRequest { id: a.id.clone(), kwant_id: kwant.into(), tool_name: tool_name.into(), details: json!({}) })
        .await;
    (a.id, rx)
}

/// Gives a bot a hand-picked face ("" = none), keeping the rest of it.
async fn set_sprite(state: &AppState, id: &str, sprite: &str) {
    let k = state.db.get_kwant(id).await.unwrap().unwrap();
    let input = UpdateKwant {
        name: k.name,
        title: k.title,
        persona_prompt: k.persona_prompt,
        soul: k.soul,
        avatar_sprite: sprite.into(),
        preferred_model: k.preferred_model,
    };
    state.db.update_kwant(id, &input).await.unwrap().unwrap();
}

fn new_bot(name: &str) -> NewKwant {
    NewKwant {
        name: name.into(),
        title: "Helper".into(),
        persona_prompt: "Helps.".into(),
        soul: String::new(),
        avatar_sprite: "ben".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    }
}

fn direct(kwant: &str, role: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: None,
        direct_kwant_id: Some(kwant.into()),
        sender_role: role.into(),
        sender_id: if role == "user" { "PlayerHero".into() } else { kwant.into() },
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    }
}

fn find<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array().unwrap().iter().find(|i| i["id"] == id).unwrap_or_else(|| panic!("{id} not in {list}"))
}

fn ids(list: &Value) -> BTreeSet<String> {
    list.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn the_glance_lists_what_needs_you_works_and_finished_and_leaks_nothing() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    // A face picked by hand, and a bot with none
    set_sprite(&state, &spec.id, "fox").await;
    set_sprite(&state, &lead.id, "").await;

    // Empty: every list there, nothing in them
    let (v, _) = glance(&app).await;
    assert_eq!(v["counts"], json!({"needs_you": 0, "working": 0, "finished_unseen": 0}));
    for k in ["needs_you", "working", "finished"] {
        assert_eq!(v[k], json!([]), "{k}");
    }
    assert!(v["now"].as_str().is_some());

    // Needs you: a question in a team chat, and approvals of every kind
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();
    let place = QuestionPlace { kwant_id: spec.id.clone(), channel_id: Some(chan.id.clone()), ..Default::default() };
    let choices: Vec<String> = (1..=8).map(|i| format!("Option {i}")).collect();
    let q = state.db.create_question(&place, "Which one ships tonight?", &choices).await.unwrap();
    let mcp = |server: &str, tool: &str, args: Value| json!({"source": "mcp", "server": server, "tool": tool, "arguments": args});
    let (plain, _r1) = approval(&state, &lead.id, "mcp__notion__create_page", mcp("notion", "create_page", json!({"title": "Notes", "api_key": SECRET}))).await;
    let mut pay = mcp("stripe", "create_invoice", json!({"amount": 500}));
    pay["payment_reason"] = json!("Sends an invoice for $5");
    let (money, _r2) = approval(&state, &lead.id, "mcp__stripe__create_invoice", pay).await;
    let mut prod = mcp("db", "query", json!({"sql": "delete from users"}));
    prod["prod_floor"] = json!({"server": "db"});
    let (floor, _r3) = approval(&state, &lead.id, "mcp__db__query", prod).await;
    let push = mcp("github", "create_or_update_file", json!({"owner": "o", "repo": "r", "path": "a.md", "content": "x", "message": "m", "branch": "main"}));
    let (pushes, _r4) = approval(&state, &lead.id, "mcp__github__create_or_update_file", push).await;
    let mut risky = mcp("notion", "delete_page", json!({"id": "p1"}));
    risky["smart_reason"] = json!("Deletes a page");
    let (high, _r5) = approval(&state, &lead.id, "mcp__notion__delete_page", risky).await;
    let shell = json!({"source": "local_shell", "tool": "shell", "arguments": {"command": format!("curl -H 'x: {SECRET}' x && git push")}});
    let (cmd, _r6) = approval(&state, &lead.id, "shell", shell).await;

    // Working: a running turn with a tool step whose command holds a key
    let (tx, _keep) = tokio::sync::broadcast::channel(16);
    let tag = TurnTag { turn_id: "turn-glance-1".into(), kwant_id: spec.id.clone(), context_id: spec.id.clone(), thread_id: None };
    let activity = TurnActivity::start(tx, tag);
    activity.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": format!("export KEY={SECRET}; ls")}) });
    state.turns.set_turn(ActiveTurn {
        turn_id: "turn-glance-1".into(),
        kwant_id: spec.id.clone(),
        context_id: spec.id.clone(),
        thread_id: None,
        state: "running".into(),
        since: Utc::now(),
        purpose: None,
        audit: None,
    });
    // The hidden decide-first round and a queued turn aren't working
    for (id, st, purpose) in [("turn-decide", "running", Some("decide".to_string())), ("turn-queued", "queued", None)] {
        state.turns.set_turn(ActiveTurn {
            turn_id: id.into(),
            kwant_id: lead.id.clone(),
            context_id: lead.id.clone(),
            thread_id: None,
            state: st.into(),
            since: Utc::now(),
            purpose,
            audit: None,
        });
    }

    // Finished: a hand-off with a result
    let (_thread, d) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: lead.id.clone(),
            specialist_kwant_id: spec.id.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(lead.id.clone()),
            origin_thread_id: None,
            team_id: None,
            task: "do the brief".into(),
            title: "Write the brief".into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    assert!(state.db.mark_delegation_running(&d.id).await.unwrap());
    let m = state.db.insert_message(&direct(&spec.id, "kwant", &format!("Done. key {SECRET}"))).await.unwrap();
    assert!(state.db.finish_delegation(&d.id, "completed", None, Some(&m.id)).await.unwrap());

    // Ask targets: the bot the person wrote to last comes first
    state.db.insert_message(&direct(&spec.id, "user", "hi")).await.unwrap();

    let (v, _) = glance(&app).await;
    println!("GLANCE_SAMPLE {}", serde_json::to_string_pretty(&v).unwrap());
    let text = v.to_string();
    assert!(!text.contains("sk-ant"), "no secret anywhere: {text}");
    assert!(!text.contains("delete from users") && !text.contains("Done. key"), "no arguments or message bodies: {text}");

    // Needs you is exactly what /api/away lists as waiting
    let (_, _, away) = send(&app, "GET", "/api/away", Req::default()).await;
    assert_eq!(ids(&v["needs_you"]), ids(&away["waiting"]));
    assert_eq!(v["counts"]["needs_you"], away["unseen"]["waiting"]);
    assert_eq!(v["counts"]["needs_you"], 7);

    let qi = find(&v["needs_you"], &q.id);
    assert_eq!(qi["kind"], "question");
    assert_eq!(qi["space_id"], "spc_default");
    assert_eq!(qi["kwant_id"], spec.id);
    assert_eq!(qi["title"], "Which one ships tonight?");
    assert_eq!(qi["approval"], Value::Null);
    assert_eq!(qi["question"]["choices"].as_array().unwrap().len(), 6);
    assert_eq!(qi["question"]["choices_total"], 8);
    assert_eq!(qi["question"]["allow_text"], true);
    assert_eq!(qi["path"], format!("/s/spc_default/c/{}?card={}", chan.id, q.id));
    for k in ["space_name", "kwant_name", "since"] {
        assert!(qi[k].as_str().is_some_and(|s| !s.is_empty()), "{k}: {qi}");
    }

    let inline = |id: &str| find(&v["needs_you"], id)["approval"]["can_allow_inline"].clone();
    assert_eq!(inline(&plain), true);
    assert_eq!(find(&v["needs_you"], &plain)["approval"]["risk"], "normal");
    for (name, id) in [("money", &money), ("production floor", &floor), ("push", &pushes), ("risky", &high), ("shell", &cmd)] {
        assert_eq!(inline(id), false, "{name}");
        let a = &find(&v["needs_you"], id)["approval"];
        assert_eq!(a["risk"], "high", "{name}");
        assert!(a["why_open"].as_str().is_some_and(|w| !w.is_empty()), "{name}: {a}");
    }
    assert_eq!(find(&v["needs_you"], &plain)["path"], format!("/s/spc_default/b/{}?card={plain}", lead.id));

    // Working: the running turn only, its step in plain words
    assert_eq!(v["counts"]["working"], 1);
    let w = &v["working"][0];
    assert_eq!(w["kwant_id"], spec.id);
    assert_eq!(w["space_id"], "spc_default");
    assert_eq!(w["path"], format!("/s/spc_default/b/{}", spec.id));
    assert!(w["step"].as_str().is_some_and(|s| !s.is_empty()), "{w}");
    // 17a: a shell step names its program, never the command text
    assert_eq!(w["step"], "Running export", "{w}");
    assert!(!w["step"].as_str().unwrap().contains('`'), "{w}");
    assert!(w["started_at"].as_str().is_some());

    // Finished: unseen, linked to its result
    assert_eq!(v["counts"]["finished_unseen"], 1);
    let f = find(&v["finished"], &d.id);
    assert_eq!(f["title"], "Write the brief");
    assert_eq!(f["unseen"], true);
    assert_eq!(f["kwant_id"], spec.id);
    assert!(f["finished_at"].as_str().is_some());
    assert!(f["check"].is_null() || ["verified", "not_checked", "failed"].contains(&f["check"]["status"].as_str().unwrap()));

    // Marking the space seen makes it seen
    let (s, _, _) = send(&app, "POST", "/api/away/seen", Req::default()).await;
    assert_eq!(s, StatusCode::OK);
    let (v2, _) = glance(&app).await;
    assert_eq!(find(&v2["finished"], &d.id)["unseen"], false);
    assert_eq!(v2["counts"]["finished_unseen"], 0);

    let targets = v["ask_targets"].as_array().unwrap();
    assert!(targets.len() <= 8 && !targets.is_empty());
    assert_eq!(targets[0]["kwant_id"], spec.id);
    assert_eq!(targets[0]["space_id"], "spc_default");

    // Every item that names a bot carries its saved face, or null when it has none
    let face = |id: &str| match id {
        i if i == spec.id => json!("fox"),
        i if i == lead.id => Value::Null,
        i => kwants.iter().find(|k| k.id == i && !k.avatar_sprite.is_empty()).map_or(Value::Null, |k| json!(k.avatar_sprite)),
    };
    for list in ["needs_you", "working", "finished", "ask_targets"] {
        let items = v[list].as_array().unwrap();
        assert!(!items.is_empty(), "{list}");
        for it in items {
            let id = it["kwant_id"].as_str().unwrap();
            assert_eq!(it.get("avatar"), Some(&face(id)), "{list}: {it}");
        }
    }
    assert_eq!(qi["avatar"], "fox");
    assert_eq!(find(&v["needs_you"], &plain)["avatar"], Value::Null);
    assert_eq!(w["avatar"], "fox");
    assert_eq!(f["avatar"], "fox");
    assert_eq!(targets[0]["avatar"], "fox");
    drop(activity);
}

#[tokio::test]
async fn an_unchanged_glance_is_a_304() {
    let (app, state, _tmp) = setup().await;
    let (_, tag) = glance(&app).await;
    assert!(tag.starts_with("W/\""), "{tag}");
    let (s, h, v) = send(&app, "GET", "/api/glance", Req { if_none_match: Some(&tag), ..Default::default() }).await;
    assert_eq!(s, StatusCode::NOT_MODIFIED, "{v}");
    assert_eq!(h["etag"].to_str().unwrap(), tag);
    assert_eq!(v, Value::Null, "no body");
    // `now` moves, the tag doesn't
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(glance(&app).await.1, tag);

    // Something new: a fresh tag and the full body
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let place = QuestionPlace { kwant_id: bot.id.clone(), direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    state.db.create_question(&place, "Go?", &["Yes".into()]).await.unwrap();
    let (s, h, v) = send(&app, "GET", "/api/glance", Req { if_none_match: Some(&tag), ..Default::default() }).await;
    assert_eq!(s, StatusCode::OK);
    let tag = h["etag"].to_str().unwrap().to_string();
    assert_ne!(tag, "");
    assert_eq!(v["counts"]["needs_you"], 1);

    // A new face is a change too
    set_sprite(&state, &bot.id, "owl").await;
    let (s, h, v) = send(&app, "GET", "/api/glance", Req { if_none_match: Some(&tag), ..Default::default() }).await;
    assert_eq!(s, StatusCode::OK);
    assert_ne!(h["etag"].to_str().unwrap(), tag);
    assert_eq!(v["needs_you"][0]["avatar"], "owl");
}

#[tokio::test]
async fn archived_spaces_are_left_out() {
    let (app, state, _tmp) = setup().await;
    let lab = state.db.create_space(&NewSpace { name: "Lab".into(), icon: "🧪".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let lab_id = SpaceId::new(&lab.id);
    let bot = state.db.create_kwant(&lab_id, &new_bot("Labby")).await.unwrap();
    let place = QuestionPlace { kwant_id: bot.id.clone(), direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    let q = state.db.create_question(&place, "Lab question?", &["Yes".into()]).await.unwrap();
    let (a, _rx) = approval(&state, &bot.id, "mcp__notion__create_page", json!({"source": "mcp", "server": "notion", "tool": "create_page", "arguments": {}})).await;
    state.db.insert_message(&direct(&bot.id, "user", "hello lab")).await.unwrap();

    let (v, _) = glance(&app).await;
    let qi = find(&v["needs_you"], &q.id);
    assert_eq!((qi["space_id"].as_str(), qi["space_name"].as_str()), (Some(lab.id.as_str()), Some("Lab")));
    assert_eq!(qi["path"], format!("/s/{}/b/{}?card={}", lab.id, bot.id, q.id));
    find(&v["needs_you"], &a);
    assert_eq!(v["counts"]["needs_you"], 2);
    assert_eq!(v["ask_targets"][0]["kwant_id"], bot.id);

    state.db.archive_space(&lab_id).await.unwrap();
    let (v, _) = glance(&app).await;
    assert_eq!(v["needs_you"], json!([]), "{v}");
    assert_eq!(v["counts"]["needs_you"], 0);
    assert!(!v.to_string().contains(&lab.id), "nothing from the archived space: {v}");
}

async fn turn_token(state: &AppState) -> String {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-glance-bot".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, &bot, Some("turn-glance-bot".into()), None, Some(scope)).await.unwrap().1
}

#[tokio::test]
async fn only_the_person_reads_the_glance_or_asks_from_it() {
    let (app, state, _tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let ask = json!({"space_id": "spc_default", "kwant_id": bot.id, "text": "hi"});

    // A bot's turn token: 403 on both, even from this machine
    let token = turn_token(&state).await;
    let (s, _, v) = send(&app, "GET", "/api/glance", Req { bearer: Some(&token), ..Default::default() }).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("person_only")), "{v}");
    let (s, _, v) = send(&app, "POST", "/api/glance/ask", Req { body: Some(ask.clone()), bearer: Some(&token), ..Default::default() }).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("person_only")), "{v}");

    // From another machine: no token 401, a wrong one 401, a foreign page's origin 403
    let (s, _, _) = send(&app, "GET", "/api/glance", Req { remote: true, ..Default::default() }).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = send(&app, "GET", "/api/glance", Req { remote: true, cookie: Some("nope"), ..Default::default() }).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = send(&app, "GET", "/api/glance", Req { remote: true, cookie: Some(TOKEN), origin: Some("https://evil.example"), ..Default::default() }).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // The desktop app: the person's cookie (or bearer), no Origin
    let (s, h, v) = send(&app, "GET", "/api/glance", Req { remote: true, cookie: Some(TOKEN), ..Default::default() }).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h["cache-control"], "private, no-cache");
    let (s, _, _) = send(&app, "GET", "/api/glance", Req { remote: true, bearer: Some(TOKEN), ..Default::default() }).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn the_desktop_app_answers_with_the_cookie_alone() {
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = kwants[0].clone();
    let rust = |body: Value| Req { body: Some(body), remote: true, cookie: Some(TOKEN), ..Default::default() };

    // Allow once on an approval the glance offers inline
    let (a, rx) = approval(&state, &bot.id, "mcp__notion__create_page", json!({"source": "mcp", "server": "notion", "tool": "create_page", "arguments": {}})).await;
    let (v, _) = glance(&app).await;
    assert_eq!(find(&v["needs_you"], &a)["approval"]["can_allow_inline"], true);
    let (s, _, body) = send(&app, "POST", &format!("/api/approvals/{a}/resolve"), rust(json!({"decision": "allow_once"}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(matches!(rx.await, Ok(ApprovalDecision::AllowOnce)));
    // Deny
    let (b, rx) = approval(&state, &bot.id, "mcp__notion__create_page", json!({"source": "mcp", "server": "notion", "tool": "create_page", "arguments": {}})).await;
    let (s, _, body) = send(&app, "POST", &format!("/api/approvals/{b}/resolve"), rust(json!({"decision": "deny"}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(matches!(rx.await, Ok(ApprovalDecision::Deny)));

    // A question: by choice index, or in the person's own words
    let place = QuestionPlace { kwant_id: bot.id.clone(), direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    let q1 = state.db.create_question(&place, "Which?", &["Red".into(), "Blue".into()]).await.unwrap();
    let q2 = state.db.create_question(&place, "Why?", &["Because".into()]).await.unwrap();
    let (s, _, body) = send(&app, "POST", &format!("/api/questions/{}/answer", q1.id), rust(json!({"choice": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _, body) = send(&app, "POST", &format!("/api/questions/{}/answer", q2.id), rust(json!({"text": "It's simpler"}))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (v, _) = glance(&app).await;
    assert_eq!(v["counts"]["needs_you"], 0, "{v}");
}

#[tokio::test]
async fn a_quick_ask_lands_in_the_bots_chat() {
    let (app, state, _tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let rust = |body: Value| Req { body: Some(body), remote: true, cookie: Some(TOKEN), ..Default::default() };

    let (s, _, v) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": "spc_default", "kwant_id": bot.id, "text": "  What's left today?  ", "client_message_id": "island-1"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["path"], format!("/s/spc_default/b/{}", bot.id));
    let id = v["user_message_id"].as_str().unwrap().to_string();
    let m = state.db.get_message(&id).await.unwrap().unwrap();
    assert_eq!((m.text.as_str(), m.direct_kwant_id.as_deref(), m.sender_role.as_str()), ("What's left today?", Some(bot.id.as_str()), "user"));
    // The same ask again (a retry) is sent once
    let (s, _, again) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": "spc_default", "kwant_id": bot.id, "text": "What's left today?", "client_message_id": "island-1"}))).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!((again["user_message_id"].as_str(), again["deduped"].as_bool()), (Some(id.as_str()), Some(true)));

    // Empty, too long, a bot from another space, an archived space
    let (s, _, v) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": "spc_default", "kwant_id": bot.id, "text": "   "}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("empty")));
    let (s, _, v) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": "spc_default", "kwant_id": bot.id, "text": "x".repeat(8_001)}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("too_long")));
    let lab = state.db.create_space(&NewSpace { name: "Lab".into(), icon: "🧪".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let (s, _, v) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": lab.id, "kwant_id": bot.id, "text": "hi"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    let lab_bot = state.db.create_kwant(&SpaceId::new(&lab.id), &new_bot("Labby")).await.unwrap();
    state.db.archive_space(&SpaceId::new(&lab.id)).await.unwrap();
    let (s, _, v) = send(&app, "POST", "/api/glance/ask", rust(json!({"space_id": lab.id, "kwant_id": lab_bot.id, "text": "hi"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
}

/// The island polls this every 5 s per place: it has to stay cheap with 30 spaces. Prints the
/// timing (`--nocapture`); the bound is loose because this is a debug build on a shared machine.
#[tokio::test]
async fn thirty_busy_spaces_glance_quickly() {
    let (app, state, _tmp) = setup().await;
    let mut keep = Vec::new();
    for i in 0..30 {
        let s = state.db.create_space(&NewSpace { name: format!("Space {i}"), icon: "📦".into(), color: None, this_machine_enabled: true }).await.unwrap();
        let sid = SpaceId::new(&s.id);
        let a = state.db.create_kwant(&sid, &new_bot(&format!("Ana {i}"))).await.unwrap();
        let b = state.db.create_kwant(&sid, &new_bot(&format!("Ben {i}"))).await.unwrap();
        let place = QuestionPlace { kwant_id: a.id.clone(), direct_kwant_id: Some(a.id.clone()), ..Default::default() };
        state.db.create_question(&place, &format!("Ship {i}?"), &["Yes".into(), "No".into()]).await.unwrap();
        keep.push(approval(&state, &b.id, "mcp__notion__create_page", json!({"source": "mcp", "server": "notion", "tool": "create_page", "arguments": {}})).await);
        for n in 0..5 {
            state.db.insert_message(&direct(&a.id, if n % 2 == 0 { "user" } else { "kwant" }, "hello")).await.unwrap();
        }
        let (_t, d) = state
            .db
            .create_delegation(&NewDelegation {
                lead_kwant_id: a.id.clone(),
                specialist_kwant_id: b.id.clone(),
                origin_channel_id: None,
                origin_kwant_id: Some(a.id.clone()),
                origin_thread_id: None,
                team_id: None,
                task: "t".into(),
                title: format!("Task {i}"),
                continue_lead: false,
                depth: 1,
                follow_up_at: None,
            })
            .await
            .unwrap();
        state.db.mark_delegation_running(&d.id).await.unwrap();
        let m = state.db.insert_message(&direct(&b.id, "kwant", "done")).await.unwrap();
        state.db.finish_delegation(&d.id, "completed", None, Some(&m.id)).await.unwrap();
    }
    let (v, tag) = glance(&app).await;
    assert_eq!(v["counts"]["needs_you"], 60, "{}", v["counts"]);
    let mut full = Vec::new();
    let mut not_modified = Vec::new();
    let mut spaces_list = Vec::new();
    let mut one_away = Vec::new();
    for _ in 0..20 {
        let t = std::time::Instant::now();
        glance(&app).await;
        full.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        let (s, _, _) = send(&app, "GET", "/api/glance", Req { if_none_match: Some(&tag), ..Default::default() }).await;
        not_modified.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(s, StatusCode::NOT_MODIFIED);
        // Baselines on the same machine at the same moment: one list read, one space's /api/away
        let t = std::time::Instant::now();
        let (s, _, _) = send(&app, "GET", "/api/spaces", Req::default()).await;
        spaces_list.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(s, StatusCode::OK);
        let t = std::time::Instant::now();
        let (s, _, _) = send(&app, "GET", "/api/away", Req::default()).await;
        one_away.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(s, StatusCode::OK);
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v[v.len() / 2], v[v.len() * 9 / 10])
    };
    let (f50, f90) = median(&mut full);
    let (n50, n90) = median(&mut not_modified);
    let (s50, _) = median(&mut spaces_list);
    let (a50, _) = median(&mut one_away);
    println!("GLANCE 31 spaces, 60 waiting, 30 finished: 200 median {f50:.1} ms p90 {f90:.1} ms; 304 median {n50:.1} ms p90 {n90:.1} ms; body {} bytes; baseline /api/spaces median {s50:.1} ms, one space's /api/away median {a50:.1} ms", v.to_string().len());
    assert!(f50 < 1000.0, "{f50} ms");
}

/// Rigel's round-2 risk-4 rows (#646) plus one row per 0.9.14 and 18c rule, read back through
/// `GET /api/glance`: twenty-seven steps that must never get Allow once inline, and four ordinary
/// ones that still do.
#[tokio::test]
async fn risky_steps_never_get_allow_once_and_ordinary_ones_still_do() {
    let (app, state, _tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let mcp = |server: &str, tool: &str, args: Value| json!({"source": "mcp", "server": server, "tool": tool, "arguments": args});
    let run = |c: &str| mcp("runner", "run", json!({ "command": c }));
    let rows: Vec<(&str, Value, bool)> = vec![
        ("git  push (two spaces)", run("git  push origin main"), false),
        ("git<TAB>push", run("git\tpush origin main"), false),
        ("git -C dir push", run("git -C /repo push origin main"), false),
        ("git -c k=v push", run("git -c core.x=1 push"), false),
        ("argv [git, push]", mcp("runner", "run", json!({"argv": ["git", "push", "origin", "main"]})), false),
        ("gh pr merge", run("gh pr merge 5 --squash"), false),
        ("gh release create", run("gh release create v1"), false),
        ("gh api -X PUT merge", run("gh api -X PUT repos/o/r/pulls/5/merge"), false),
        ("git send-pack", run("git send-pack origin main"), false),
        ("npm publish", run("npm publish"), false),
        ("cargo publish", run("cargo publish"), false),
        ("docker push", run("docker push img"), false),
        ("curl POST git/refs", run("curl -X POST https://api.github.com/repos/o/r/git/refs"), false),
        ("rm -rf /", run("rm -rf /"), false),
        ("bitbucket pr_action merge", mcp("bitbucket", "pr_action", json!({"action": "merge"})), false),
        ("publish_site", mcp("netlify", "publish_site", json!({})), false),
        ("deploy", mcp("vercel", "deploy", json!({})), false),
        ("send_email", mcp("gmail", "send_email", json!({})), false),
        ("delete_page", mcp("notion", "delete_page", json!({})), false),
        // 0.9.14 hardening, one row per rule
        ("key commandLine", mcp("notes", "save_note", json!({"commandLine": "ls -la"})), false),
        ("key pushToOrigin", mcp("notes", "save_note", json!({"pushToOrigin": true})), false),
        ("transfer_funds", mcp("bank", "transfer_funds", json!({})), false),
        ("method DELETE", mcp("http", "http_request", json!({"url": "https://api.example.com/items/1", "method": "DELETE"})), false),
        // 18c, one row per new rule
        ("key input", mcp("x", "do", json!({"input": "git reset --hard HEAD~5"})), false),
        ("tool create_branch", mcp("gh", "create_branch", json!({"branch": "x"})), false),
        ("action delete", mcp("linear", "manage_issue", json!({"action": "delete", "id": "A-1"})), false),
        ("a URL with a body and no method", mcp("http", "fetch", json!({"url": "https://api.example.com/items/1", "body": {"a": 1}})), false),
        ("method GET", mcp("http", "http_request", json!({"url": "https://api.example.com/items/1", "method": "GET"})), true),
        (
            "a file write inside the bot's folder",
            json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "arguments": {"file_path": "notes/todo.md", "content": "Buy milk\nCall Ana"}}),
            true,
        ),
        ("a read-only fetch", mcp("fetch", "fetch", json!({"url": "https://example.com/docs", "max_length": 5000})), true),
        ("a page that mentions Pushkin", mcp("notion", "create_page", json!({"title": "Pushkin reading list", "parent": {"page_id": "abc"}})), true),
    ];
    assert_eq!(rows.iter().filter(|r| !r.2).count(), 27);
    let person = |body: Value| Req { body: Some(body), remote: true, cookie: Some(TOKEN), ..Default::default() };
    // Ten at a time keeps them under the Needs-you cap; each batch is denied before the next
    for batch in rows.chunks(10) {
        let mut made = Vec::new();
        for (name, details, _) in batch {
            let tool = details["tool"].as_str().unwrap().to_string();
            made.push((name, approval(&state, &bot.id, &tool, details.clone()).await));
        }
        let (v, _) = glance(&app).await;
        for ((name, (id, _rx)), (_, _, inline)) in made.iter().zip(batch) {
            let a = &find(&v["needs_you"], id)["approval"];
            assert_eq!(a["can_allow_inline"], *inline, "{name}: {a}");
            assert_eq!(a["risk"], if *inline { "normal" } else { "high" }, "{name}");
            assert_eq!(a["why_open"].is_null(), *inline, "{name}: {a}");
        }
        for (_, (id, _rx)) in &made {
            let (s, _, body) = send(&app, "POST", &format!("/api/approvals/{id}/resolve"), person(json!({"decision": "deny"}))).await;
            assert_eq!(s, StatusCode::OK, "{body}");
        }
    }
}

/// 21a: through the glance, ordinary reads of mail, files, posts and payments keep Allow once, and
/// a name that acts on them still says it acts outside kwant.
#[tokio::test]
async fn reads_of_mail_files_and_payments_keep_allow_once_on_the_island() {
    use kwnt::server::glance::OUTWARD;
    let (app, state, _tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let mcp = |server: &str, tool: &str, args: Value| json!({"source": "mcp", "server": server, "tool": tool, "arguments": args});
    let rows: Vec<(&str, Value, bool)> = vec![
        ("GMAIL_FETCH_EMAILS", mcp("composio", "GMAIL_FETCH_EMAILS", json!({"max_results": 10})), true),
        ("DROPBOX_LIST_FILES", mcp("composio", "DROPBOX_LIST_FILES", json!({"path": "/"})), true),
        ("WORDPRESS_GET_POST", mcp("composio", "WORDPRESS_GET_POST", json!({"post_id": 7})), true),
        ("get_payment_status", mcp("stripe", "get_payment_status", json!({"id": "pi_1"})), true),
        ("rollout_status", mcp("ci", "rollout_status", json!({})), true),
        ("GMAIL_SEND_EMAIL", mcp("composio", "GMAIL_SEND_EMAIL", json!({"to": "a@example.com"})), false),
        ("find_and_share_file", mcp("drive", "find_and_share_file", json!({})), false),
        ("pay_invoice", mcp("stripe", "pay_invoice", json!({"id": "in_1"})), false),
        ("get_post with a POST body", mcp("x", "get_post", json!({"url": "https://api.example.com/posts", "method": "POST", "body": {"t": "x"}})), false),
    ];
    let mut made = Vec::new();
    for (name, details, _) in &rows {
        let tool = details["tool"].as_str().unwrap().to_string();
        made.push((name, approval(&state, &bot.id, &tool, details.clone()).await));
    }
    let (v, _) = glance(&app).await;
    for ((name, (id, _rx)), (_, _, inline)) in made.iter().zip(&rows) {
        let a = &find(&v["needs_you"], id)["approval"];
        assert_eq!(a["can_allow_inline"], *inline, "{name}: {a}");
        if *inline {
            assert!(a["why_open"].is_null(), "{name}: {a}");
        } else {
            assert_eq!(a["why_open"], OUTWARD, "{name}: {a}");
        }
    }
}

/// 18c: every row Rigel's r10/r11 still allowed at 253bacc9 ("Still allowed after 253bacc9" in
/// desktop-island/qa-server-review.md), with the reason the island now gives, and the steps that
/// must keep Allow once. Same details as his probes. The Telegram column is `ships_or_runs`: only
/// the commit and branch names reach Telegram's floor; the rest stay island-only, as 18b chose.
#[test]
fn the_rows_still_allowed_after_253bacc9_now_need_the_full_card() {
    use kwnt::server::glance::{inline_refusal, ships_or_runs, OUTWARD, PUSHES, RUNS};
    let mcp = |server: &str, tool: &str, args: Value| json!({"source": "mcp", "server": server, "tool": tool, "arguments": args});
    let url = "https://api.example.com/items/1";
    // (row, details, island reason, Telegram's floor)
    type Row = (&'static str, Value, &'static str, Option<&'static str>);
    let rows: Vec<Row> = vec![
        // Command text under keys no older rule read
        ("B10 input, quoted push", mcp("x", "do", json!({"input": "git pu''sh origin main"})), RUNS, None),
        ("B11 input, base64", mcp("x", "do", json!({"input": "echo Z2l0IHB1c2g= | base64 -d | sh"})), RUNS, None),
        ("B12 input, env-var split", mcp("x", "do", json!({"input": "p=pu; git ${p}sh origin main"})), RUNS, None),
        ("B13 line, rm -rf", mcp("x", "do", json!({"line": "rm -rf /home"})), RUNS, None),
        ("B17 statement on invoke", mcp("x", "invoke", json!({"statement": "DROP TABLE users"})), RUNS, None),
        ("B52 input, gh pr close", mcp("x", "do", json!({"input": "gh pr close 5"})), RUNS, None),
        ("B53 input, git reset --hard", mcp("x", "do", json!({"input": "git reset --hard HEAD~5"})), RUNS, None),
        ("H06 code_snippet", mcp("x", "do", json!({"code_snippet": "import os; os.system('x')"})), RUNS, None),
        ("H07 arguments_list", mcp("x", "do", json!({"arguments_list": ["rm", "-rf", "/"]})), RUNS, None),
        ("H08 statement, nested", mcp("x", "invoke", json!({"q": {"statement": "DROP TABLE users"}})), RUNS, None),
        // Tool names
        ("B34 broadcast", mcp("x", "broadcast", json!({})), OUTWARD, None),
        ("B54 ship_it", mcp("x", "ship_it", json!({"env": "prod"})), OUTWARD, None),
        ("B54 rollout", mcp("x", "rollout", json!({})), OUTWARD, None),
        ("B55 commit_files", mcp("gh", "commit_files", json!({"branch": "main"})), PUSHES, Some(PUSHES)),
        ("B56 create_commit", mcp("gh", "create_commit", json!({"message": "m"})), PUSHES, Some(PUSHES)),
        ("B56 create_branch", mcp("gh", "create_branch", json!({"branch": "x"})), PUSHES, Some(PUSHES)),
        ("B56 GITHUB_CREATE_A_BRANCH", mcp("composio", "GITHUB_CREATE_A_BRANCH", json!({})), PUSHES, Some(PUSHES)),
        // Verbs under other keys, and a body with no verb
        ("H01 action: delete", mcp("linear", "manage_issue", json!({"action": "delete", "id": "A-1"})), OUTWARD, None),
        ("H02 operation: DELETE", mcp("x", "do", json!({"operation": "DELETE", "id": "1"})), OUTWARD, None),
        ("H02 op: remove_item", mcp("x", "do", json!({"op": "remove_item"})), OUTWARD, None),
        ("H03 request_verb DELETE", mcp("http", "fetch", json!({"url": url, "request_verb": "DELETE"})), OUTWARD, None),
        ("H04 URL + body, no method", mcp("http", "fetch", json!({"url": url, "body": {"a": 1}})), OUTWARD, None),
        ("H04 URL + json, no method", mcp("http", "fetch", json!({"url": url, "json": "{}"})), OUTWARD, None),
        ("H05 X-HTTP-Method-Override", mcp("http", "fetch", json!({"url": url, "method": "GET", "headers": {"X-HTTP-Method-Override": "DELETE"}})), OUTWARD, None),
    ];
    assert_eq!(rows.len(), 24);
    for (name, d, why, telegram) in &rows {
        assert_eq!(inline_refusal(d), Some(*why), "{name}");
        assert_eq!(ships_or_runs(d), *telegram, "{name}: Telegram");
    }

    // Still Allow once: the brief's keep-inline steps, and the near misses of each new rule
    let keep: Vec<(&str, Value)> = vec![
        ("a Write in the bot's folder", json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "arguments": {"file_path": "notes/todo.md", "content": "Buy milk\nCall Ana"}})),
        ("a fetch of a URL", mcp("fetch", "fetch", json!({"url": "https://example.com/docs", "max_length": 5000}))),
        ("a GET of a URL", mcp("http", "fetch", json!({"url": url, "method": "GET"}))),
        ("Notion Pushkin reading list", mcp("notion", "create_page", json!({"title": "Pushkin reading list", "parent": {"page_id": "abc"}}))),
        ("description + transcript_id", mcp("linear", "create_issue", json!({"description": "x", "transcript_id": "t"}))),
        ("method median, not HTTP", mcp("stats", "summarize", json!({"method": "median"}))),
        // near misses
        ("get_commit reads", mcp("github", "get_commit", json!({"sha": "abc"}))),
        ("list_commits / list_branches read", mcp("github", "list_branches", json!({"repo": "r"}))),
        ("GITHUB_GET_A_BRANCH reads", mcp("composio", "GITHUB_GET_A_BRANCH", json!({"branch": "main"}))),
        ("membership is not ship", mcp("x", "get_membership", json!({}))),
        ("a transaction key is not an action", mcp("ledger", "summarize", json!({"transaction": "delete me"}))),
        ("action: summarize", mcp("x", "do", json!({"action": "summarize"}))),
        ("a body with no URL", mcp("notes", "save_note", json!({"body": "x"}))),
        ("headline / deadline keys", mcp("notion", "create_page", json!({"headline": "x", "deadline": "Fri"}))),
    ];
    for (name, d) in &keep {
        assert_eq!(inline_refusal(d), None, "{name}");
    }
}
