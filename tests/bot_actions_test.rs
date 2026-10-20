//! Agents changing the app from a chat through their own tools: routines, bots, settings, their
//! space and channels, each change shown as a card the person can undo; and everything that stays
//! with the person (secrets, sign-ins, approvals, MCP installs, the computer, other spaces).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Channel, Kwant, Message},
        spaces::SpaceId,
        Database,
    },
    mcp::{self, McpServerInput},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
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
    other: Kwant,
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
    Env { app: create_router(state.clone()), bot: kwants[0].clone(), other: kwants[1].clone(), general, state, _tmp: tmp }
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

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc("tools/call", json!({"name": name, "arguments": args})))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
        v["result"]["isError"].as_bool().unwrap_or(false),
    )
}

async fn ok(e: &Env, token: &str, name: &str, args: Value) -> String {
    let (text, err) = tool(e, token, name, args.clone()).await;
    assert!(!err, "{name} {args} failed: {text}");
    text
}

async fn refused(e: &Env, token: &str, name: &str, args: Value) -> String {
    let (text, err) = tool(e, token, name, args.clone()).await;
    assert!(err, "{name} {args} should be refused: {text}");
    text
}

/// A turn of `bot` in #general with its own tools; returns the turn token.
async fn turn_in(e: &Env, bot: &Kwant, origin: TurnOrigin, turn_id: &str) -> String {
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn turn(e: &Env, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.id.clone()), ..Default::default() };
    turn_in(e, &e.bot, origin, turn_id).await
}

async fn actions(e: &Env) -> Vec<Value> {
    let (status, v) = call(&e.app, "GET", "/api/bot-actions", None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

async fn last_action(e: &Env) -> Value {
    actions(e).await.into_iter().next().expect("an action")
}

async fn messages(e: &Env, chat: &str) -> Vec<Message> {
    e.state.db.list_messages(chat, 200).await.unwrap()
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().map(|s| serde_json::from_str(s).unwrap()).unwrap_or(Value::Null)
}

async fn routines(e: &Env) -> Vec<Value> {
    let (_, v) = call(&e.app, "GET", "/api/routines", None, None).await;
    v.as_array().unwrap().clone()
}

async fn set_timezone(e: &Env, tz: &str) {
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"timezone": tz}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn routine_is_created_in_this_chat_as_this_bot_with_a_card_and_managed_through_tools() {
    let e = setup().await;
    let token = turn(&e, "t-routine").await;

    // No time zone known yet: the agent is told to pass one or ask, and nothing is created
    let text = refused(&e, &token, "routine_create", json!({"name": "Morning motivation", "instructions": "Send me motivation",
        "schedule": {"type": "daily", "time": "07:00"}})).await;
    assert!(text.contains("time zone"), "{text}");
    assert!(routines(&e).await.is_empty());

    set_timezone(&e, "Europe/Berlin").await;
    let text = ok(&e, &token, "routine_create", json!({"name": "Morning motivation",
        "instructions": "Send the person a short, upbeat motivation message for the day.",
        "schedule": {"type": "daily", "time": "07:00"}})).await;
    assert!(text.contains("daily 07:00 Europe/Berlin") && text.contains("#general"), "{text}");
    let list = routines(&e).await;
    assert_eq!(list.len(), 1);
    let r = &list[0];
    assert_eq!(r["kwant_id"], e.bot.id.as_str());
    assert_eq!(r["channel_id"], e.general.id.as_str(), "delivers into the chat it was made in");
    assert_eq!(r["schedule"]["tz"], "Europe/Berlin");
    assert_eq!(r["enabled"], true);
    let routine_id = r["id"].as_str().unwrap().to_string();

    // The card: a message from the bot in this chat, with what Undo/Edit/Pause need
    let a = last_action(&e).await;
    assert_eq!((a["action"].as_str(), a["status"].as_str(), a["turn_id"].as_str()), (Some("routine_create"), Some("applied"), Some("t-routine")));
    assert_eq!(a["kwant_id"], e.bot.id.as_str());
    let card = messages(&e, &e.general.id).await.into_iter().find(|m| Some(m.id.as_str()) == a["message_id"].as_str()).expect("card");
    assert_eq!((card.sender_role.as_str(), card.sender_id.as_str()), ("kwant", e.bot.name.as_str()));
    assert!(card.text.contains("Morning motivation") && card.text.contains("daily 07:00 Europe/Berlin"), "{}", card.text);
    let m = meta(&card);
    assert_eq!(m["kind"], "bot_action");
    assert_eq!(m["action_id"], a["id"]);
    assert_eq!(m["target_kind"], "routine");
    assert_eq!(m["target_id"], routine_id.as_str());
    assert_eq!(m["can_undo"], true);
    assert_eq!(m["needs_confirmation"], false);
    assert_eq!(m["details"]["schedule_text"], "daily 07:00 Europe/Berlin");
    assert_eq!(m["details"]["bot_name"], e.bot.name.as_str());

    // List, update, pause, resume, by name
    let text = ok(&e, &token, "routine_list", json!({})).await;
    assert!(text.contains("Morning motivation") && text.contains("daily 07:00 Europe/Berlin"), "{text}");
    let text = ok(&e, &token, "routine_update", json!({"routine": "Morning motivation", "schedule": {"type": "weekly", "time": "08:15",
        "weekdays": ["mon", "fri"], "tz": "America/New_York"}})).await;
    assert!(text.contains("weekly on mon, fri at 08:15 America/New_York"), "{text}");
    ok(&e, &token, "routine_pause", json!({"routine": routine_id})).await;
    assert_eq!(routines(&e).await[0]["enabled"], false);
    ok(&e, &token, "routine_resume", json!({"routine": routine_id})).await;
    assert_eq!(routines(&e).await[0]["enabled"], true);

    // Run now: the routine's message lands in #general (the bot's reply needs an engine)
    let text = ok(&e, &token, "routine_run_now", json!({"routine": "Morning motivation"})).await;
    assert!(text.contains("#general"), "{text}");
    let mut posted = false;
    for _ in 0..50 {
        if messages(&e, &e.general.id).await.iter().any(|m| m.sender_id == "Routine: Morning motivation") {
            posted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(posted, "run_now posts the routine into its chat");
    assert_eq!(last_action(&e).await["card"]["can_undo"], false, "a run can't be undone");

    // Delete, then the person undoes it: the routine comes back
    ok(&e, &token, "routine_delete", json!({"routine": routine_id})).await;
    assert!(routines(&e).await.is_empty());
    let del = last_action(&e).await;
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", del["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "undone");
    assert_eq!(v["card"]["status"], "undone");
    let back = routines(&e).await;
    assert_eq!(back.len(), 1);
    assert_eq!(back[0]["name"], "Morning motivation");
    assert_eq!(back[0]["channel_id"], e.general.id.as_str());
    assert_eq!(v["target_id"], back[0]["id"], "the action now points at the recreated routine");
    let card = messages(&e, &e.general.id).await.into_iter().find(|m| Some(m.id.as_str()) == del["message_id"].as_str()).unwrap();
    assert_eq!(meta(&card)["status"], "undone", "the card follows");

    // Undoing the create removes it
    let create = actions(&e).await.into_iter().find(|a| a["action"] == "routine_create").unwrap();
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    // The routine it made was deleted and recreated, so that row is gone: a conflict, not a crash
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn schedule_and_input_errors_are_tool_errors_and_create_nothing() {
    let e = setup().await;
    let token = turn(&e, "t-bad").await;
    let make = |schedule: Value| json!({"name": "R", "instructions": "do it", "schedule": schedule});
    for (schedule, needle) in [
        (json!({"type": "daily", "time": "07:00", "tz": "Mars/Base"}), "time zone"),
        (json!({"type": "daily", "time": "25:00", "tz": "UTC"}), "time"),
        (json!({"type": "weekly", "time": "07:00", "weekdays": [], "tz": "UTC"}), "day"),
        (json!({"type": "interval", "every_minutes": 1}), "minutes"),
        (json!({"type": "once", "at": "2001-01-01T07:00:00Z"}), "future"),
        (json!({"type": "hourly"}), "not valid"),
        (json!("every morning"), "object"),
    ] {
        let text = refused(&e, &token, "routine_create", make(schedule.clone())).await;
        assert!(text.to_lowercase().contains(needle), "{schedule}: {text}");
    }
    let text = refused(&e, &token, "routine_create", json!({"name": "", "instructions": "x", "schedule": {"type": "interval", "every_minutes": 60}})).await;
    assert!(text.contains("name"), "{text}");
    let text = refused(&e, &token, "routine_create", json!({"name": "R", "instructions": "x", "schedule": {"type": "interval", "every_minutes": 60}, "bot": "Nobody"})).await;
    assert!(text.contains("no bot"), "{text}");
    let text = refused(&e, &token, "routine_create", json!({"name": "R", "instructions": "x", "schedule": {"type": "interval", "every_minutes": 60}, "channel": "nope"})).await;
    assert!(text.contains("no channel"), "{text}");
    refused(&e, &token, "routine_pause", json!({"routine": "missing"})).await;
    assert!(routines(&e).await.is_empty());
    assert!(actions(&e).await.is_empty());

    // Another bot and its direct chat can be the target
    let text = ok(&e, &token, "routine_create", json!({"name": "R", "instructions": "x", "schedule": {"type": "interval", "every_minutes": 60},
        "bot": e.other.name, "channel": "direct"})).await;
    assert!(text.contains(&format!("direct chat with {}", e.other.name)), "{text}");
    let r = &routines(&e).await[0];
    assert_eq!((r["kwant_id"].as_str(), r["channel_id"].as_str()), (Some(e.other.id.as_str()), None));
}

#[tokio::test]
async fn bots_are_created_and_updated_and_the_person_undoes_it() {
    let e = setup().await;
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(&e, &e.bot, origin, "t-bots").await;

    refused(&e, &token, "bot_create", json!({"name": "Two words", "title": "x", "instructions": "x"})).await;
    let text = refused(&e, &token, "bot_create", json!({"name": "Chef", "title": "x", "instructions": "x", "model": "nonsense-model"})).await;
    assert!(text.contains("Unknown model"), "{text}");
    let text = refused(&e, &token, "bot_create", json!({"name": e.other.name, "title": "x", "instructions": "x"})).await;
    assert!(text.contains("not created"), "{text}");

    let text = ok(&e, &token, "bot_create", json!({"name": "Chef", "title": "Recipe helper",
        "instructions": "You suggest recipes from what is in the fridge.", "model": "claude", "avatar": "chef"})).await;
    assert!(text.contains("Created bot Chef"), "{text}");
    let chef = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().into_iter().find(|k| k.name == "Chef").expect("Chef");
    assert_eq!((chef.title.as_str(), chef.persona_prompt.as_str(), chef.preferred_model.as_str()),
        ("Recipe helper", "You suggest recipes from what is in the fridge.", "claude"));
    let create = last_action(&e).await;
    assert_eq!(create["target_id"], chef.id.as_str());
    // Its card is in the direct chat where it was asked
    let card = messages(&e, &e.bot.id).await.into_iter().find(|m| Some(m.id.as_str()) == create["message_id"].as_str()).expect("card");
    assert!(card.text.contains("created bot Chef") || card.text.contains("Created bot Chef"), "{}", card.text);

    // Update: fields and exactly which installed tool servers it has
    let input: McpServerInput = serde_json::from_value(json!({"name": "notes", "transport": "http", "url": "http://127.0.0.1:9/mcp"})).unwrap();
    let server = e.state.db.create_mcp_server(&SpaceId::default_space(), &mcp::build_server(input, None).unwrap()).await.unwrap();
    let text = refused(&e, &token, "bot_update", json!({"bot": "Chef", "mcp_servers": ["github"]})).await;
    assert!(text.contains("mcp_suggest"), "{text}");
    let text = ok(&e, &token, "bot_update", json!({"bot": "Chef", "title": "Head chef", "mcp_servers": []})).await;
    assert!(text.contains("title") && text.contains("mcp_server_ids"), "{text}");
    let chef_now = e.state.db.get_kwant(&chef.id).await.unwrap().unwrap();
    assert_eq!(chef_now.title, "Head chef");
    assert!(e.state.db.get_kwant_mcp_server_ids(&chef.id).await.unwrap().is_empty());
    let update = last_action(&e).await;

    // Undoing the create while a newer change is applied: 409, undo that first
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("newer"), "{v}");

    // A bot's or turn's token can't undo; the person can
    for bearer in [token.as_str(), "made-up-token"] {
        let (status, _) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", update["id"].as_str().unwrap()), Some(bearer), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", update["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let chef_back = e.state.db.get_kwant(&chef.id).await.unwrap().unwrap();
    assert_eq!(chef_back.title, "Recipe helper");
    assert_eq!(e.state.db.get_kwant_mcp_server_ids(&chef.id).await.unwrap(), vec![server.id.clone()]);

    let (status, _) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(e.state.db.get_kwant(&chef.id).await.unwrap().is_none(), "undo removed Chef");
    // Twice is a conflict
    let (status, _) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn deleting_a_bot_waits_for_the_person() {
    let e = setup().await;
    let token = turn(&e, "t-del").await;
    ok(&e, &token, "bot_create", json!({"name": "Temp", "title": "t", "instructions": "t"})).await;
    let temp = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().into_iter().find(|k| k.name == "Temp").unwrap();
    let text = ok(&e, &token, "bot_delete", json!({"bot": "Temp", "reason": "not needed"})).await;
    assert!(text.contains("confirm"), "{text}");
    assert!(e.state.db.get_kwant(&temp.id).await.unwrap().is_some(), "nothing happens before the person confirms");
    let req = last_action(&e).await;
    assert_eq!((req["status"].as_str(), req["card"]["needs_confirmation"].as_bool()), (Some("pending"), Some(true)));

    // A bot can't confirm its own request
    let (status, _) = call(&e.app, "POST", &format!("/api/bot-actions/{}/confirm", req["id"].as_str().unwrap()), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/confirm", req["id"].as_str().unwrap()), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "confirmed");
    assert!(e.state.db.get_kwant(&temp.id).await.unwrap().is_none());

    // A bot with history can't be deleted at all
    e.state.db.insert_message(&kwnt::db::models::NewMessage {
        channel_id: Some(e.general.id.clone()), direct_kwant_id: None, sender_role: "kwant".into(), sender_id: e.other.name.clone(),
        text: "hello".into(), reasoning_content: None, tool_calls: None, tokens_used: 0,
    }).await.unwrap();
    let text = refused(&e, &token, "bot_delete", json!({"bot": e.other.name})).await;
    assert!(text.contains("can't be deleted") || text.contains("history"), "{text}");

    // Archiving the space: declined, nothing changes
    let text = refused(&e, &token, "space_archive", json!({})).await;
    assert!(text.contains("Default"), "{text}");
}

#[tokio::test]
async fn non_secret_settings_change_and_undo() {
    let e = setup().await;
    let token = turn(&e, "t-settings").await;
    let text = ok(&e, &token, "settings_get", json!({})).await;
    assert!(text.contains("max_turns_per_agent") && !text.contains("api_key"), "{text}");
    let text = ok(&e, &token, "settings_update", json!({"settings": {"max_turns_per_agent": 5, "timezone": "Europe/Kiev"}})).await;
    assert!(text.contains("max_turns_per_agent = 5") && text.contains("Europe/Kyiv"), "{text}");
    let (_, s) = call(&e.app, "GET", "/api/settings", None, None).await;
    assert_eq!((s["max_turns_per_agent"].as_u64(), s["timezone"].as_str()), (Some(5), Some("Europe/Kyiv")));

    let text = refused(&e, &token, "settings_update", json!({"settings": {"timezone": "Nowhere/City"}})).await;
    assert!(text.contains("time zone"), "{text}");
    let text = refused(&e, &token, "settings_update", json!({"settings": {"max_turns_global": 0}})).await;
    assert!(text.contains("max_turns_global"), "{text}");

    let a = last_action(&e).await;
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", a["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, s) = call(&e.app, "GET", "/api/settings", None, None).await;
    assert_eq!(s["timezone"], "", "undo clears the time zone it set");
    assert_eq!(s["max_turns_per_agent"].as_u64(), Some(3), "back to the default");
}

#[tokio::test]
async fn what_stays_with_the_person_is_refused_with_where_to_do_it() {
    let e = setup().await;
    let token = turn(&e, "t-refuse").await;
    let (_, before) = call(&e.app, "GET", "/api/settings", None, None).await;
    for (key, value, place) in [
        ("anthropic_api_key", json!("sk-ant-xyz"), "Accounts"),
        ("notify_webhook_url", json!("https://ntfy.sh/x"), "Notifications"),
        ("ollama_url", json!("http://evil:11434"), "Accounts"),
        ("engine_login", json!("claude"), "sign in"),
        ("auth_mode", json!("none"), "sign in to kwnt"),
        ("auto_approve", json!(true), "answer the card"),
        ("computer_mode", json!("local"), "Computer for browsing and commands"),
        ("agent_tools", json!(false), "Only the person"),
        ("mcp_servers", json!([]), "mcp_suggest"),
    ] {
        let text = refused(&e, &token, "settings_update", json!({"settings": {key: value}})).await;
        assert!(text.contains(place), "{key}: {text}");
        assert!(text.contains("Nothing was changed"), "{key}: {text}");
    }
    // A refused key in a batch blocks the allowed one too
    refused(&e, &token, "settings_update", json!({"settings": {"max_turns_global": 2, "xai_api_key": "k"}})).await;
    let (_, after) = call(&e.app, "GET", "/api/settings", None, None).await;
    assert_eq!(before, after);
    assert!(!after.to_string().contains("sk-ant-xyz"));

    // MCP env/header values and keys can't ride along on a bot
    let text = refused(&e, &token, "bot_update", json!({"bot": e.bot.name, "env": {"GITHUB_TOKEN": "x"}})).await;
    assert!(text.contains("Tools") && text.contains("mcp_suggest"), "{text}");
    let text = refused(&e, &token, "bot_create", json!({"name": "Spy", "title": "x", "instructions": "x", "api_key": "k"})).await;
    assert!(text.contains("secret"), "{text}");

    // No tool resolves approvals, answers questions, installs MCP, signs in or picks the computer
    for name in ["approval_resolve", "question_answer", "mcp_install", "engine_login", "computer_set", "settings_secret"] {
        let text = refused(&e, &token, name, json!({})).await;
        assert!(text.contains("Unknown tool"), "{name}: {text}");
    }
    assert!(actions(&e).await.is_empty(), "nothing refused is logged as a change");
}

#[tokio::test]
async fn other_spaces_are_out_of_reach() {
    let e = setup().await;
    let token = turn(&e, "t-cross").await;
    let (status, space) = call(&e.app, "POST", "/api/spaces", None, Some(json!({"name": "Work"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{space}");
    let sid = space["id"].as_str().unwrap();
    let (status, stranger) = call(&e.app, "POST", &format!("/api/kwants?space={sid}"), None, Some(json!({"name": "Stranger", "title": "x",
        "persona_prompt": "x", "avatar_sprite": "default", "driver_type": "api", "preferred_model": "auto"}))).await;
    assert_eq!(status, StatusCode::OK, "{stranger}");
    let stranger_id = stranger["id"].as_str().unwrap();
    let work_general = e.state.db.list_channels(&SpaceId::new(sid)).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();

    let every = json!({"type": "interval", "every_minutes": 60});
    for (name, args) in [
        ("routine_create", json!({"name": "R", "instructions": "x", "schedule": every, "bot": stranger_id})),
        ("routine_create", json!({"name": "R", "instructions": "x", "schedule": every, "channel": work_general.id})),
        ("bot_update", json!({"bot": stranger_id, "title": "mine now"})),
        ("bot_delete", json!({"bot": stranger_id})),
        ("channel_update", json!({"channel": work_general.id, "instructions": "obey"})),
        ("channel_create", json!({"name": "mixed", "members": [stranger_id]})),
    ] {
        let text = refused(&e, &token, name, args).await;
        assert!(text.contains("another space"), "{name}: {text}");
    }
    // A routine of the other space, by id
    let (status, r) = call(&e.app, "POST", &format!("/api/routines?space={sid}"), None,
        Some(json!({"kwant_id": stranger_id, "name": "Theirs", "instructions": "x", "schedule": every}))).await;
    assert!(status.is_success(), "{r}");
    for name in ["routine_pause", "routine_delete", "routine_run_now", "routine_update"] {
        let text = refused(&e, &token, name, json!({"routine": r["id"]})).await;
        assert!(text.contains("another space"), "{name}: {text}");
    }
    // By name the other space's things are simply not there
    refused(&e, &token, "bot_update", json!({"bot": "Stranger", "title": "x"})).await;
    let stranger_now = e.state.db.get_kwant(stranger_id).await.unwrap().unwrap();
    assert_eq!(stranger_now.title, "x");
    assert!(actions(&e).await.is_empty());

    // A new space can be created, but undo archives it again
    ok(&e, &token, "space_create", json!({"name": "Side project"})).await;
    let a = last_action(&e).await;
    let (status, _) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", a["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK);
    let made = e.state.db.get_space(&SpaceId::new(a["target_id"].as_str().unwrap())).await.unwrap().unwrap();
    assert!(made.archived_at.is_some());
}

#[tokio::test]
async fn one_turn_makes_at_most_ten_changes() {
    let e = setup().await;
    let token = turn(&e, "t-limit").await;
    for i in 0..10 {
        ok(&e, &token, "channel_update", json!({"channel": "general", "instructions": format!("rule {i}")})).await;
    }
    let text = refused(&e, &token, "channel_update", json!({"channel": "general", "instructions": "rule 11"})).await;
    assert!(text.contains("10 changes"), "{text}");
    assert_eq!(e.state.db.channel_instructions(&e.general.id).await.unwrap().as_deref(), Some("rule 9"));
    // Reading is still fine, and the next turn may change things again
    ok(&e, &token, "routine_list", json!({})).await;
    let next = turn(&e, "t-limit-2").await;
    ok(&e, &next, "channel_update", json!({"channel": "general", "instructions": ""})).await;
    assert_eq!(actions(&e).await.len(), 11);
    let cards = messages(&e, &e.general.id).await.into_iter().filter(|m| meta(m)["kind"] == "bot_action").count();
    assert_eq!(cards, 11);
}

#[tokio::test]
async fn card_text_follows_the_decision_and_monthly_schedules_read_naturally() {
    let e = setup().await;
    let token = turn(&e, "t-words").await;
    set_timezone(&e, "Europe/Kyiv").await;

    // A monthly rule reads as words in the tool reply, the card text and its details
    let text = ok(&e, &token, "routine_create", json!({"name": "Budget review", "instructions": "Review the budget.",
        "schedule": {"type": "monthly", "days": [1, "last"], "time": "09:00"}})).await;
    assert!(text.contains("monthly on the 1st and last day at 09:00 Europe/Kyiv"), "{text}");
    let create = last_action(&e).await;
    let card_of = |a: &Value| a["message_id"].as_str().unwrap().to_string();
    let find = |list: Vec<Message>, id: String| list.into_iter().find(|m| m.id == id).expect("card");
    let card = find(messages(&e, &e.general.id).await, card_of(&create));
    assert!(card.text.starts_with("Done: ") && card.text.contains("monthly on the 1st and last day at 09:00"), "{}", card.text);
    assert_eq!(meta(&card)["details"]["schedule_text"], "monthly on the 1st and last day at 09:00 Europe/Kyiv");

    // After Undo the message itself says so, not only its meta (history, notifications, bots)
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", create["id"].as_str().unwrap()), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let card = find(messages(&e, &e.general.id).await, card_of(&create));
    assert!(card.text.starts_with("Undone: Created routine 'Budget review'"), "{}", card.text);
    assert!(!card.text.contains("Done: "), "{}", card.text);

    // Confirmed and declined requests read that way too
    for (name, route, lead) in [("Temp1", "confirm", "Confirmed: "), ("Temp2", "decline", "Declined: ")] {
        ok(&e, &token, "bot_create", json!({"name": name, "title": "t", "instructions": "t"})).await;
        ok(&e, &token, "bot_delete", json!({"bot": name, "reason": "not needed"})).await;
        let req = last_action(&e).await;
        let card = find(messages(&e, &e.general.id).await, card_of(&req));
        assert!(card.text.starts_with("Needs your OK: "), "{}", card.text);
        let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/{route}", req["id"].as_str().unwrap()), None, None).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let card = find(messages(&e, &e.general.id).await, card_of(&req));
        assert!(card.text.starts_with(lead), "{}", card.text);
    }
}

#[tokio::test]
async fn switching_bots_tools_off_is_refused_directly() {
    let e = setup().await;
    let token = turn(&e, "t-tools").await;
    for key in ["agent_tools", "agent_tools_enabled", "tools"] {
        let text = refused(&e, &token, "settings_update", json!({"settings": {key: false}})).await;
        assert!(text.contains("Refused") && text.contains("App settings") && !text.contains("no setting"), "{key}: {text}");
    }
    assert!(actions(&e).await.is_empty());
}
