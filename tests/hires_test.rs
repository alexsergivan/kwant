//! Bots proposing to hire a new bot (`propose_hire`) and the person deciding: Deny, Accept once,
//! Always accept. A proposal creates nothing; only the person decides; decisions and the policy
//! are remembered per space and shown to the space's bots on their next turn.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Channel, Kwant, Message, NewMessage},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{self, SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: Channel,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    Env { app: create_router(state.clone()), bot: kwants[0].clone(), general, state, _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
        v["result"]["isError"].as_bool().unwrap_or(false),
    )
}

async fn ok(e: &Env, token: &str, args: Value) -> String {
    let (text, err) = tool(e, token, "propose_hire", args.clone()).await;
    assert!(!err, "propose_hire {args} failed: {text}");
    text
}

async fn refused(e: &Env, token: &str, args: Value) -> String {
    let (text, err) = tool(e, token, "propose_hire", args.clone()).await;
    assert!(err, "propose_hire {args} should be refused: {text}");
    text
}

/// A turn of `bot` in `channel` with its own tools; returns the turn token.
async fn turn_of(e: &Env, bot: &Kwant, channel: &str, turn_id: &str) -> String {
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(channel.into()), ..Default::default() }),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn turn(e: &Env, turn_id: &str) -> String {
    turn_of(e, &e.bot, &e.general.id.clone(), turn_id).await
}

fn specialist(name: &str, role: &str) -> Value {
    json!({
        "name": name,
        "role": role,
        "instructions": format!("You are {name}, the {role}."),
        "reason": "A specialist with its own instructions does this better and every week.",
        "evidence": {"basis": "specialist", "needs": ["instructions", "schedule"], "gap": "I have no standing instructions or weekly schedule for this."},
    })
}

async fn hires(e: &Env, space: &str) -> Vec<Value> {
    let (status, v) = call(&e.app, "GET", &format!("/api/hires?space={space}"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

async fn latest(e: &Env) -> Value {
    hires(e, "spc_default").await.into_iter().next().expect("a proposal")
}

async fn bot_named(e: &Env, space: &str, name: &str) -> Option<Kwant> {
    e.state.db.list_kwants(&SpaceId::new(space)).await.unwrap().into_iter().find(|k| k.name == name)
}

async fn messages(e: &Env, chat: &str) -> Vec<Message> {
    e.state.db.list_messages(chat, 200).await.unwrap()
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().map(|s| serde_json::from_str(s).unwrap()).unwrap_or(Value::Null)
}

async fn card_of(e: &Env, hire: &Value) -> (Message, Value) {
    let m = messages(e, &e.general.id).await.into_iter().find(|m| meta(m)["hire_id"] == hire["id"]).expect("a card");
    let v = meta(&m);
    (m, v)
}

async fn next_turn_context(e: &Env) -> String {
    let bot = e.state.db.get_kwant(&e.bot.id).await.unwrap().unwrap();
    continuity::prompt_block(&e.state, &bot, None, None, true).await
}

async fn person_message(e: &Env, channel: &str, text: &str) -> String {
    let m = NewMessage {
        channel_id: Some(channel.into()),
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    e.state.db.insert_message(&m).await.unwrap().id
}

#[tokio::test]
async fn a_proposal_needs_evidence_and_creates_nothing() {
    let e = setup().await;
    let before = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().len();
    let token = turn(&e, "t-ev").await;

    // No evidence, an unknown basis, thin specialist evidence, too few repeated requests: refused
    let mut none = specialist("Chef", "Recipe planner");
    none.as_object_mut().unwrap().remove("evidence");
    let text = refused(&e, &token, none).await;
    assert!(text.contains("Nothing was proposed"), "{text}");
    for evidence in [
        json!({}),
        json!({"basis": "vibes"}),
        json!({"basis": "specialist", "needs": [], "gap": "I have no standing instructions for recipes."}),
        json!({"basis": "specialist", "needs": ["instructions"], "gap": "short"}),
        json!({"basis": "specialist", "needs": ["a pony"], "gap": "I have no standing instructions for recipes."}),
        json!({"basis": "repeated", "similar_requests": []}),
    ] {
        let mut args = specialist("Chef", "Recipe planner");
        args["evidence"] = evidence.clone();
        let text = refused(&e, &token, args).await;
        assert!(text.contains("Nothing was proposed"), "{evidence}: {text}");
    }
    // "Repeated" counts the person's own requests in this space, each once
    let a = person_message(&e, &e.general.id, "plan dinners").await;
    let b = person_message(&e, &e.general.id, "plan lunches").await;
    let mut args = specialist("Chef", "Recipe planner");
    args["evidence"] = json!({"basis": "repeated", "similar_requests": [a, b, a]});
    let text = refused(&e, &token, args.clone()).await;
    assert!(text.contains("at least 3"), "{text}");
    assert!(hires(&e, "spc_default").await.is_empty());

    // With evidence: a pending proposal and a card, and still no new bot
    let c = person_message(&e, &e.general.id, "plan breakfasts").await;
    args["evidence"] = json!({"basis": "repeated", "similar_requests": [a, b, c]});
    let text = ok(&e, &token, args).await;
    assert!(text.contains("Nothing is created") && text.contains("carry on"), "{text}");
    assert_eq!(e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().len(), before);
    assert!(bot_named(&e, "spc_default", "Chef").await.is_none());
    let h = latest(&e).await;
    assert_eq!(h["status"], "pending");
    assert_eq!(h["turn_id"], "t-ev");
    assert_eq!(h["kwant_id"], json!(e.bot.id));
    let (m, card) = card_of(&e, &h).await;
    assert_eq!(m.sender_id, e.bot.name);
    assert_eq!(card["kind"], "hire_proposal");
    assert_eq!(card["choices"], json!(["deny", "accept_once", "always_accept"]));
    assert_eq!(card["evidence"]["similar_requests"].as_array().unwrap().len(), 3);
    assert_eq!(card["team"]["name"], "core-engineering", "defaults to the proposer's team");
    assert_eq!(card["can_undo"], false);

    // One proposal per turn
    let text = refused(&e, &token, specialist("Taxman", "Tax advisor")).await;
    assert!(text.contains("one per turn"), "{text}");
    // Keys and existing kinds of bots are refused
    let token = turn(&e, "t-ev2").await;
    let mut secret = specialist("Taxman", "Tax advisor");
    secret["env"] = json!({"API_KEY": "x"});
    assert!(refused(&e, &token, secret).await.contains("no keys"));
    let text = refused(&e, &token, specialist("Planner", "Recipe planner")).await;
    assert!(text.contains("already waiting"), "{text}");
    let text = refused(&e, &token, specialist("Arch2", "Systems Architect")).await;
    assert!(text.contains("already does this kind of work"), "{text}");
}

#[tokio::test]
async fn only_the_person_decides_and_accept_once_creates_the_bot_in_the_team() {
    let e = setup().await;
    let token = turn(&e, "t-acc").await;
    ok(&e, &token, specialist("Chef", "Recipe planner")).await;
    let h = latest(&e).await;
    let id = h["id"].as_str().unwrap();

    // A bot's turn token can't decide, set the policy, or deny
    for (method, uri, body) in [
        ("POST", format!("/api/hires/{id}/accept"), json!({})),
        ("POST", format!("/api/hires/{id}/accept"), json!({"always": true})),
        ("POST", format!("/api/hires/{id}/deny"), json!({})),
        ("PUT", "/api/spaces/spc_default/hire-policy".to_string(), json!({"policy": "auto"})),
    ] {
        let (status, v) = call(&e.app, method, &uri, Some(&token), Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
    assert!(bot_named(&e, "spc_default", "Chef").await.is_none());
    assert_eq!(latest(&e).await["status"], "pending");
    // ... and a bot can read the policy
    let (text, err) = tool(&e, &token, "hire_status", json!({})).await;
    assert!(!err && text.contains("ask the person each time") && text.contains("Chef"), "{text}");

    // Accept once
    let (status, v) = call(&e.app, "POST", &format!("/api/hires/{id}/accept"), None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "accepted");
    let chef = bot_named(&e, "spc_default", "Chef").await.expect("Chef was created");
    assert_eq!(chef.title, "Recipe planner");
    assert_eq!(v["hired_kwant_id"], json!(chef.id));
    let team = e.state.db.get_team_by_name(&SpaceId::default_space(), "core-engineering").await.unwrap().unwrap();
    assert!(team.member_kwants.contains(&chef.id), "joined the team");
    let team_channel = e.state.db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == "team-core-engineering");
    if let Some(c) = team_channel {
        assert!(c.member_kwants.contains(&chef.id), "joined the team's channel");
    }
    let (m, card) = card_of(&e, &h).await;
    assert_eq!(card["status"], "accepted");
    assert_eq!(card["can_undo"], true);
    assert_eq!(card["choices"], json!([]));
    assert!(m.text.starts_with("Hired Chef"), "{}", m.text);
    // Deciding twice: conflict
    let (status, _) = call(&e.app, "POST", &format!("/api/hires/{id}/deny"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Accept once doesn't change the policy: the next proposal asks again
    let (_, p) = call(&e.app, "GET", "/api/spaces/spc_default/hire-policy", None, None).await;
    assert_eq!(p["policy"], "ask");
    let token = turn(&e, "t-acc2").await;
    ok(&e, &token, specialist("Taxman", "Tax advisor")).await;
    assert_eq!(latest(&e).await["status"], "pending");
    assert!(bot_named(&e, "spc_default", "Taxman").await.is_none());

    // Undo through the bot action removes the hired bot and the card says so
    let action = card["action_id"].as_str().unwrap();
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{action}/undo"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(bot_named(&e, "spc_default", "Chef").await.is_none());
    let team = e.state.db.get_team_by_name(&SpaceId::default_space(), "core-engineering").await.unwrap().unwrap();
    assert!(!team.member_kwants.contains(&chef.id));
    let (m, card) = card_of(&e, &h).await;
    assert_eq!(card["status"], "undone");
    assert_eq!(card["can_undo"], false);
    assert!(m.text.starts_with("Hire undone"), "{}", m.text);
}

#[tokio::test]
async fn always_accept_hires_at_once_with_undo_up_to_the_daily_cap() {
    let e = setup().await;
    let token = turn(&e, "t-al0").await;
    ok(&e, &token, specialist("Chef", "Recipe planner")).await;
    let id = latest(&e).await["id"].as_str().unwrap().to_string();
    let (status, v) = call(&e.app, "POST", &format!("/api/hires/{id}/accept"), None, Some(json!({"always": true}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["card"]["policy"], "auto");
    let (_, p) = call(&e.app, "GET", "/api/spaces/spc_default/hire-policy", None, None).await;
    assert_eq!(p["policy"], "auto");
    assert_eq!(p["auto_hires_last_24h"], 0, "the person's own accept isn't automatic");

    // The next three are hired at once, each with an Undo card
    for (i, (name, role)) in [("Taxman", "Tax advisor"), ("Coach", "Fitness coach"), ("Gardener", "Garden designer")].iter().enumerate() {
        let token = turn(&e, &format!("t-al{}", i + 1)).await;
        let text = ok(&e, &token, specialist(name, role)).await;
        assert!(text.contains("Hired") && text.contains("always accept"), "{text}");
        let h = latest(&e).await;
        assert_eq!(h["status"], "auto_accepted");
        assert!(bot_named(&e, "spc_default", name).await.is_some(), "{name} created");
        let (m, card) = card_of(&e, &h).await;
        assert_eq!(card["can_undo"], true);
        assert_eq!(card["auto"], true);
        assert!(m.text.contains("always accept"), "{}", m.text);
    }
    // The fourth in 24 hours asks instead
    let token = turn(&e, "t-al4").await;
    let text = ok(&e, &token, specialist("Painter", "Portrait painter")).await;
    assert!(text.contains("Nothing is created"), "{text}");
    let h = latest(&e).await;
    assert_eq!(h["status"], "pending");
    assert!(bot_named(&e, "spc_default", "Painter").await.is_none());
    let (_, card) = card_of(&e, &h).await;
    assert!(card["note"].as_str().unwrap_or("").contains("already hired automatically"), "{card}");
    let (_, p) = call(&e.app, "GET", "/api/spaces/spc_default/hire-policy", None, None).await;
    assert_eq!(p["auto_hires_last_24h"], 3);

    // Undo an automatic hire
    let auto = hires(&e, "spc_default").await.into_iter().find(|h| h["name"] == "Coach").unwrap();
    let action = auto["action_id"].as_str().unwrap();
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{action}/undo"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(bot_named(&e, "spc_default", "Coach").await.is_none());

    // The person turns it back to asking
    let (status, p) = call(&e.app, "PUT", "/api/spaces/spc_default/hire-policy", None, Some(json!({"policy": "ask"}))).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(p["policy"], "ask");
    let (status, _) = call(&e.app, "PUT", "/api/spaces/spc_default/hire-policy", None, Some(json!({"policy": "yolo"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&e.app, "GET", "/api/spaces/spc_nope/hire-policy", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deny_is_remembered_in_its_space_only() {
    let e = setup().await;
    let token = turn(&e, "t-d1").await;
    ok(&e, &token, specialist("Chef", "Recipe planner")).await;
    let h = latest(&e).await;
    let (status, v) = call(&e.app, "POST", &format!("/api/hires/{}/deny", h["id"].as_str().unwrap()), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "denied");
    assert!(bot_named(&e, "spc_default", "Chef").await.is_none());
    let (m, card) = card_of(&e, &h).await;
    assert!(m.text.starts_with("Not hired"), "{}", m.text);
    assert_eq!(card["choices"], json!([]));

    // The same kind of bot under another name is refused in this space, even with "always accept"
    let (_, _) = call(&e.app, "PUT", "/api/spaces/spc_default/hire-policy", None, Some(json!({"policy": "auto"}))).await;
    for (i, (name, role)) in [("Cook", "recipes planner"), ("Chef", "Kitchen helper"), ("Menu", "Weekly recipe planner")].iter().enumerate() {
        let token = turn(&e, &format!("t-d2{i}")).await;
        let text = refused(&e, &token, specialist(name, role)).await;
        assert!(text.contains("turned down"), "{name}: {text}");
    }
    // A different kind of bot is fine
    let token = turn(&e, "t-d3").await;
    ok(&e, &token, specialist("Taxman", "Tax advisor")).await;
    // When the person asks for one, it may be proposed again, but it asks: never automatic
    let token = turn(&e, "t-d4").await;
    let mut asked = specialist("Cook", "Recipe planner");
    asked["requested_by_person"] = json!(true);
    ok(&e, &token, asked).await;
    let h = latest(&e).await;
    assert_eq!(h["status"], "pending");
    assert!(bot_named(&e, "spc_default", "Cook").await.is_none());

    // Another space doesn't inherit the denial
    let (status, space) = call(&e.app, "POST", "/api/spaces", None, Some(json!({"name": "Home"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{space}");
    let sid = space["id"].as_str().unwrap();
    let (status, other) = call(&e.app, "POST", &format!("/api/kwants?space={sid}"), None, Some(json!({"name": "Helper", "title": "Generalist",
        "persona_prompt": "x", "avatar_sprite": "default", "driver_type": "api", "preferred_model": "auto"}))).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    let other = e.state.db.get_kwant(other["id"].as_str().unwrap()).await.unwrap().unwrap();
    let home_general = e.state.db.list_channels(&SpaceId::new(sid)).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    let token = turn_of(&e, &other, &home_general.id, "t-home").await;
    ok(&e, &token, specialist("Chef", "Recipe planner")).await;
    let home = hires(&e, sid).await;
    assert_eq!(home.len(), 1);
    assert_eq!(home[0]["status"], "pending", "Home's policy is its own: ask");
    let (_, p) = call(&e.app, "GET", &format!("/api/spaces/{sid}/hire-policy"), None, None).await;
    assert_eq!(p["denied"], json!([]));
    // A team from another space is refused
    let token = turn_of(&e, &other, &home_general.id, "t-home2").await;
    let mut cross = specialist("Taxman", "Tax advisor");
    cross["team"] = json!(e.state.db.get_team_by_name(&SpaceId::default_space(), "core-engineering").await.unwrap().unwrap().id);
    assert!(refused(&e, &token, cross).await.contains("another space"));

    // The person clears the denials: the kind can be proposed again
    let (status, p) = call(&e.app, "PUT", "/api/spaces/spc_default/hire-policy", None, Some(json!({"policy": "ask", "clear_denials": true}))).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(p["denied"], json!([]));
}

#[tokio::test]
async fn the_policy_and_decisions_appear_in_the_next_turn() {
    let e = setup().await;
    let ctx = next_turn_context(&e).await;
    assert!(ctx.contains("[Hiring new bots in this space]") && ctx.contains("ask the person each time"), "{ctx}");
    assert!(ctx.contains("yourself first"), "{ctx}");

    let token = turn(&e, "t-c1").await;
    ok(&e, &token, specialist("Chef", "Recipe planner")).await;
    let ctx = next_turn_context(&e).await;
    assert!(ctx.contains("Chef (Recipe planner): waiting for the person"), "{ctx}");

    let id = latest(&e).await["id"].as_str().unwrap().to_string();
    call(&e.app, "POST", &format!("/api/hires/{id}/deny"), None, None).await;
    let token = turn(&e, "t-c2").await;
    ok(&e, &token, specialist("Taxman", "Tax advisor")).await;
    let id = latest(&e).await["id"].as_str().unwrap().to_string();
    call(&e.app, "POST", &format!("/api/hires/{id}/accept"), None, Some(json!({"always": true}))).await;

    let ctx = next_turn_context(&e).await;
    assert!(ctx.contains("always accept"), "{ctx}");
    assert!(ctx.contains("3 of 3 automatic hires left"), "{ctx}");
    assert!(ctx.contains("Chef (Recipe planner): DENIED") && ctx.contains("unless the person asks"), "{ctx}");
    assert!(ctx.contains("Taxman (Tax advisor): hired (the person accepted once)"), "{ctx}");

    // Another space's bots see none of it
    let (_, space) = call(&e.app, "POST", "/api/spaces", None, Some(json!({"name": "Home"}))).await;
    let sid = space["id"].as_str().unwrap();
    let (_, other) = call(&e.app, "POST", &format!("/api/kwants?space={sid}"), None, Some(json!({"name": "Helper", "title": "Generalist",
        "persona_prompt": "x", "avatar_sprite": "default", "driver_type": "api", "preferred_model": "auto"}))).await;
    let other = e.state.db.get_kwant(other["id"].as_str().unwrap()).await.unwrap().unwrap();
    let ctx = continuity::prompt_block(&e.state, &other, None, None, true).await;
    assert!(ctx.contains("ask the person each time") && !ctx.contains("Chef") && !ctx.contains("Taxman"), "{ctx}");
}
