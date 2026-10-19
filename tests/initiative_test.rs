//! Calendar schedules (monthly, cron) and follow-ups a bot promises and schedules itself, delivered
//! into the same chat or thread with the exchange it follows up on; and the "how to work" prompt.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Channel, Kwant, Message, MessageExtra, NewMessage},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{self, SelfScope, TurnOrigin},
        create_router, mcp_runtime, scheduler, AppState,
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


async fn routines(e: &Env) -> Vec<Value> {
    let (_, v) = call(&e.app, "GET", "/api/routines", None, None).await;
    v.as_array().unwrap().clone()
}

async fn set_timezone(e: &Env, tz: &str) {
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"timezone": tz}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
}


fn say(chat_channel: Option<&str>, direct: Option<&str>, role: &str, sender: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: chat_channel.map(Into::into),
        direct_kwant_id: direct.map(Into::into),
        sender_role: role.into(),
        sender_id: sender.into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    }
}

#[tokio::test]
async fn calendar_rules_are_real_monthly_and_cron_routines_not_one_run() {
    let e = setup().await;
    let token = turn(&e, "t-cal").await;
    set_timezone(&e, "Europe/Berlin").await;

    // "Remind me on the 1st of each month": a monthly rule that keeps running, in the person's zone
    let text = ok(&e, &token, "routine_create", json!({"name": "Budget review", "instructions": "Remind the person to review the budget.",
        "schedule": {"type": "monthly", "time": "09:00", "days": [1]}})).await;
    assert!(text.contains("monthly on the 1st at 09:00 Europe/Berlin"), "{text}");
    let r = routines(&e).await.into_iter().find(|r| r["name"] == "Budget review").unwrap();
    assert_eq!(r["schedule"]["type"], "monthly");
    assert_eq!(r["schedule"]["tz"], "Europe/Berlin", "the zone comes from the settings");
    assert_eq!(r["schedule_text"], "monthly on the 1st at 09:00 Europe/Berlin");
    let next = chrono::DateTime::parse_from_rfc3339(r["next_run_at"].as_str().unwrap()).unwrap();
    let local = next.with_timezone(&chrono_tz_offset("Europe/Berlin", next));
    assert_eq!(local.format("%d %H:%M").to_string(), "01 09:00", "{next}");

    // The last day of the month, and a cron rule
    let text = ok(&e, &token, "routine_create", json!({"name": "Month end", "instructions": "Close the books.",
        "schedule": {"type": "monthly", "time": "17:30", "days": ["last"]}})).await;
    assert!(text.contains("the last day"), "{text}");
    let text = ok(&e, &token, "routine_create", json!({"name": "Second Monday", "instructions": "Plan the sprint.",
        "schedule": {"type": "cron", "expression": "0 9 * * MON#2", "tz": "America/New_York"}})).await;
    assert!(text.contains("cron \"0 9 * * MON#2\" America/New_York"), "{text}");
    let card = last_action(&e).await;
    assert_eq!(card["card"]["details"]["schedule_text"], "cron \"0 9 * * MON#2\" America/New_York");

    // Bad rules are tool errors, and nothing is created
    for (schedule, needle) in [
        (json!({"type": "cron", "expression": "0 9 30 2 *", "tz": "UTC"}), "never"),
        (json!({"type": "cron", "expression": "* * * * *", "tz": "UTC"}), "minutes"),
        (json!({"type": "cron", "expression": "0 9 * *", "tz": "UTC"}), "five"),
        (json!({"type": "cron", "expression": "@daily", "tz": "UTC"}), "five"),
        (json!({"type": "monthly", "time": "09:00", "days": [32], "tz": "UTC"}), "day"),
        (json!({"type": "monthly", "time": "09:00", "days": [], "tz": "UTC"}), "day"),
    ] {
        let text = refused(&e, &token, "routine_create", json!({"name": "Bad", "instructions": "x", "schedule": schedule.clone()})).await;
        assert!(text.to_lowercase().contains(needle), "{schedule}: {text}");
    }
    assert_eq!(routines(&e).await.len(), 3);

    // The schema the agent sees offers the calendar rules
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(rpc("tools/list", json!({})))).await;
    let tools = v["result"]["tools"].as_array().unwrap();
    let create = tools.iter().find(|t| t["name"] == "routine_create").unwrap().to_string();
    assert!(create.contains("monthly") && create.contains("cron"), "{create}");
    assert!(tools.iter().any(|t| t["name"] == "follow_up_create"));
}

/// The UTC offset `tz` has at `t`, as a fixed offset (keeps the test free of a tz crate).
fn chrono_tz_offset(tz: &str, t: chrono::DateTime<chrono::FixedOffset>) -> chrono::FixedOffset {
    assert_eq!(tz, "Europe/Berlin");
    let utc = t.with_timezone(&chrono::Utc);
    // Central European Summer Time runs from the last Sunday of March to the last Sunday of October
    use chrono::Datelike;
    let last_sunday = |month: u32| {
        let mut d = chrono::NaiveDate::from_ymd_opt(utc.year(), month + 1, 1).unwrap().pred_opt().unwrap();
        while d.weekday() != chrono::Weekday::Sun {
            d = d.pred_opt().unwrap();
        }
        d.and_hms_opt(1, 0, 0).unwrap().and_utc()
    };
    let summer = utc >= last_sunday(3) && utc < last_sunday(10);
    chrono::FixedOffset::east_opt(if summer { 7200 } else { 3600 }).unwrap()
}

#[tokio::test]
async fn a_follow_up_comes_back_to_the_same_chat_as_the_same_bot_and_can_be_undone() {
    let e = setup().await;
    let person = e.state.db.insert_message(&say(Some(&e.general.id), None, "user", "user", "I start a new workout plan on Monday")).await.unwrap();
    let token = turn(&e, "t-fu").await;
    let at = (chrono::Utc::now() + chrono::Duration::days(3)).to_rfc3339();

    let text = ok(&e, &token, "follow_up_create", json!({"at": at, "check": "Ask how the first workouts went and whether to adjust the plan."})).await;
    assert!(text.contains("#general"), "{text}");
    let r = &routines(&e).await[0];
    assert_eq!(r["kwant_id"], e.bot.id.as_str());
    assert_eq!(r["channel_id"], e.general.id.as_str());
    assert_eq!(r["schedule"]["type"], "once");
    assert_eq!(r["context_message_id"], person.id.as_str(), "it remembers what it follows up on");
    assert!(r["name"].as_str().unwrap().starts_with("Follow-up"), "{r}");

    // A card the person can undo
    let a = last_action(&e).await;
    assert_eq!((a["action"].as_str(), a["status"].as_str()), (Some("routine_create"), Some("applied")));
    assert!(a["summary"].as_str().unwrap().contains("follow-up"), "{a}");
    let (status, v) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", a["id"].as_str().unwrap()), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(routines(&e).await.is_empty());

    // Input errors
    let text = refused(&e, &token, "follow_up_create", json!({"at": "2001-01-01T00:00:00Z", "check": "x"})).await;
    assert!(text.contains("future"), "{text}");
    let text = refused(&e, &token, "follow_up_create", json!({"at": "next monday", "check": "x"})).await;
    assert!(text.contains("RFC 3339"), "{text}");
    refused(&e, &token, "follow_up_create", json!({"at": at})).await;
    assert!(routines(&e).await.is_empty());
}

#[tokio::test]
async fn a_follow_up_promised_in_a_thread_runs_in_that_thread_with_the_earlier_exchange() {
    let e = setup().await;
    let thread = e.state.db.create_thread(&e.bot.id, "Workout plan", "chat", None, "user").await.unwrap();
    let in_thread = MessageExtra { thread_id: Some(thread.id.clone()), meta: None };
    let person = e.state.db.insert_message_with(&say(None, None, "user", "user", "I start a new workout plan on Monday: 3x a week, 5k runs."), &in_thread).await.unwrap();
    e.state.db.insert_message_with(&say(None, None, "kwant", &e.bot.name, "Great plan. I'll check in Thursday."), &in_thread).await.unwrap();
    let origin = TurnOrigin { thread_id: Some(thread.id.clone()), direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(&e, &e.bot, origin, "t-thread").await;
    let at = (chrono::Utc::now() + chrono::Duration::days(3)).to_rfc3339();

    let text = ok(&e, &token, "follow_up_create", json!({"at": at, "check": "Ask how the first runs went.", "name": "Workout check-in"})).await;
    assert!(text.contains("Workout plan"), "names the thread: {text}");
    let r = &routines(&e).await[0];
    assert_eq!(r["thread_id"], thread.id.as_str(), "delivered into the thread, not the channel");
    assert_eq!(r["channel_id"], Value::Null);
    assert_eq!(r["context_message_id"], person.id.as_str());
    let row = e.state.db.get_routine(r["id"].as_str().unwrap()).await.unwrap().unwrap();

    // While the exchange is in the turn's own history, nothing extra is added
    assert_eq!(scheduler::follow_up_context(&e.state, &row, &e.bot, "none").await, "");

    // Once it has scrolled out, the run gets a compact excerpt of it, from this thread only
    e.state.db.insert_message(&say(Some(&e.general.id), None, "user", "user", "SECRET from general")).await.unwrap();
    for i in 0..25 {
        e.state.db.insert_message_with(&say(None, None, "user", "user", &format!("later chatter {i}")), &in_thread).await.unwrap();
    }
    let ctx = scheduler::follow_up_context(&e.state, &row, &e.bot, "none").await;
    assert!(ctx.contains("follow-up you scheduled"), "{ctx}");
    assert!(ctx.contains("I start a new workout plan on Monday") && ctx.contains("I'll check in Thursday"), "{ctx}");
    assert!(!ctx.contains("SECRET"), "never another chat: {ctx}");
    assert!(ctx.lines().filter(|l| l.starts_with("- ")).count() <= 6, "{ctx}");
    assert!(ctx.len() < 6 * 700 + 300, "compact: {}", ctx.len());

    // Running it posts into the thread, as the routine, not into a channel or the direct chat
    let (status, v) = call(&e.app, "POST", &format!("/api/routines/{}/run", row.id), None, None).await;
    assert!(status.is_success(), "{v}");
    let mut posted = None;
    for _ in 0..50 {
        if let Some(m) = messages(&e, &thread.id).await.into_iter().find(|m| m.sender_id == "Routine: Workout check-in") {
            posted = Some(m);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let posted = posted.expect("the run posts into the thread");
    assert_eq!((posted.channel_id.as_deref(), posted.direct_kwant_id.as_deref()), (None, None));
    assert!(!messages(&e, &e.bot.id).await.iter().any(|m| m.sender_id == "Routine: Workout check-in"));
}

#[tokio::test]
async fn routines_only_deliver_into_the_bots_own_threads_and_their_own_chat() {
    let e = setup().await;
    let theirs = e.state.db.create_thread(&e.other.id, "Not yours", "chat", None, "user").await.unwrap();
    let elsewhere = e.state.db.insert_message(&say(None, Some(&e.other.id), "user", "user", "private to the other bot")).await.unwrap();
    let at = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let base = json!({"kwant_id": e.bot.id, "name": "R", "instructions": "x", "schedule": {"type": "once", "at": at}});

    let mut body = base.clone();
    body["thread_id"] = json!(theirs.id);
    let (status, v) = call(&e.app, "POST", "/api/routines", None, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "another bot's thread: {v}");

    let mut body = base.clone();
    body["context_message_id"] = json!(elsewhere.id);
    let (status, v) = call(&e.app, "POST", "/api/routines", None, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a message from another chat: {v}");
    assert!(routines(&e).await.is_empty());

    // Its own thread is fine
    let mine = e.state.db.create_thread(&e.bot.id, "Mine", "chat", None, "user").await.unwrap();
    let mut body = base.clone();
    body["thread_id"] = json!(mine.id);
    let (status, v) = call(&e.app, "POST", "/api/routines", None, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["thread_id"], mine.id.as_str());
    assert_eq!(v["channel_id"], Value::Null, "a thread delivery has no channel");
}

#[tokio::test]
async fn bots_with_tools_are_told_how_to_work_and_bots_without_are_not() {
    let e = setup().await;
    let with = continuity::prompt_block(&e.state, &e.bot, None, None, true).await;
    let without = continuity::prompt_block(&e.state, &e.bot, None, None, false).await;
    assert!(with.contains("[How to work]") && with.contains("memory_add") && with.contains("follow_up_create"), "{with}");
    assert!(!without.contains("[How to work]"));
    let section = &with[with.find("[How to work]").unwrap()..];
    let section = &section[..section[1..].find("\n\n[").map_or(section.len(), |i| i + 1)];
    // Proactive extras and check-ins, but never ones that cost money or reach outside, and no asking for reversible ones
    for words in ["Be proactive", "shopping list", "a few days in", "Don't ask permission", "End by naming every extra", "I also added a shopping list", "list or tips in your reply", "Undo", "10 changes", "costs money", "Never redo what the person undid", "a sample of what it will send", "saved notes don't"] {
        assert!(section.contains(words), "{words}: {section}");
    }
    // Bots' own tools can only be refused by name: the prompt says where the person switches them
    assert!(with.contains("App settings (gear at the bottom of the space rail) → \"Bots can use their own tools\""), "{with}");
    assert!(section.len() < 1700, "keep it tight: {} bytes", section.len());
}
