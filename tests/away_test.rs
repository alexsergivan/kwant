//! "While you were away" (batch 3): `GET /api/away` and `POST /api/away/seen`.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use kwnt::{
    broker::{ApprovalDecision, ApprovalRequest, PermissionBroker},
    config::Config,
    db::{
        continuity::NewRoutine,
        models::{McpServer, NewApproval, NewChannel, NewMessage},
        questions::QuestionPlace,
        threads::NewDelegation,
        Database,
    },
    router::LayaRouter,
    server::{create_router, mcp_runtime, turns::ActiveTurn, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use sqlx::sqlite::SqlitePoolOptions;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const SECRET: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

async fn call(app: &Router, method: &str, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn get(app: &Router, uri: &str) -> Value {
    let (s, v) = call(app, "GET", uri, &[]).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

/// Sets a timestamp column the way kwnt writes it (sqlx's encoding), so comparisons line up.
async fn set_time(state: &AppState, table: &str, column: &str, id: &str, t: DateTime<Utc>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", state.config.data_dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query(&format!("UPDATE {table} SET {column} = ? WHERE id = ?"))
        .bind(t)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

fn reply(kwant: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: None,
        direct_kwant_id: Some(kwant.into()),
        sender_role: "kwant".into(),
        sender_id: kwant.into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    }
}

/// A delegation from `lead` to `specialist` left in `status` (with `error`, or a result message).
async fn delegation(state: &AppState, lead: &str, specialist: &str, title: &str, status: &str, text: Option<&str>) -> (String, String) {
    let (thread, d) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: lead.into(),
            specialist_kwant_id: specialist.into(),
            origin_channel_id: None,
            origin_kwant_id: Some(lead.into()),
            origin_thread_id: None,
            team_id: None,
            task: format!("do {title}"),
            title: title.into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    assert!(state.db.mark_delegation_running(&d.id).await.unwrap());
    match status {
        "running" => {}
        "completed" => {
            let m = state.db.insert_message(&reply(specialist, text.unwrap_or(""))).await.unwrap();
            assert!(state.db.finish_delegation(&d.id, "completed", None, Some(&m.id)).await.unwrap());
        }
        s => assert!(state.db.finish_delegation(&d.id, s, text, None).await.unwrap()),
    }
    (d.id, thread.id)
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
}

fn find<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array().unwrap().iter().find(|i| i["id"] == id).unwrap_or_else(|| panic!("{id} missing from {list}"))
}

fn assert_newest_first(list: &Value) {
    let ts: Vec<&str> = list.as_array().unwrap().iter().map(|i| i["timestamp"].as_str().unwrap()).collect();
    assert!(ts.windows(2).all(|w| w[0] >= w[1]), "not newest first: {ts:?}");
}

fn rfc(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[tokio::test]
async fn away_lists_every_kind_filters_caps_and_marks_seen() {
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let chan = state
        .db
        .create_channel(&kwnt::db::spaces::SpaceId::default_space(), &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();
    let now = Utc::now();

    // Empty, never seen: looks back 7 days
    let v = get(&app, "/api/away").await;
    assert_eq!(v["waiting"], json!([]));
    assert_eq!(v["last_seen"], Value::Null);
    let since = DateTime::parse_from_rfc3339(v["since"].as_str().unwrap()).unwrap();
    assert!((now - Duration::days(7) - since.with_timezone(&Utc)).num_seconds().abs() < 5);
    assert_eq!(v["unseen"], json!({"waiting": 0, "finished": 0, "failed": 0, "total": 0}));

    // Waiting: an open question, an answered one (hidden), a live approval and a stale one
    let place = QuestionPlace {
        kwant_id: spec.id.clone(),
        channel_id: Some(chan.id.clone()),
        ..Default::default()
    };
    let q = state.db.create_question(&place, "Ship tonight?", &["Yes".into(), "No".into()]).await.unwrap();
    let answered = state.db.create_question(&place, "Old one?", &["A".into()]).await.unwrap();
    assert!(state.db.close_question(&answered.id, "answered", Some("A")).await.unwrap());
    set_time(&state, "questions", "created_at", &q.id, now - Duration::days(30)).await;
    let approval = |tool: &str| NewApproval {
        turn_id: "turn-gone".into(),
        kwant_id: lead.id.clone(),
        tool_name: format!("mcp__mail__{tool}"),
        parameters: json!({"server": "mail", "tool": tool, "arguments": {"to": "me@example.com", "api_key": "hidden-value", "command": "mysql -u bob password=hunter2"}}),
    };
    let live = state.db.save_approval(&approval("send")).await.unwrap();
    let _rx = state
        .broker
        .request_approval(ApprovalRequest {
            id: live.id.clone(),
            kwant_id: lead.id.clone(),
            tool_name: live.tool_name.clone(),
            details: json!({}),
        })
        .await;
    let stale = state.db.save_approval(&approval("delete")).await.unwrap();

    // Finished: a delegation with a result, a successful routine run; one too old for the default window
    let (done, done_thread) = delegation(&state, &lead.id, &spec.id, "Write the brief", "completed", Some("## Brief\nAll three sections drafted.")).await;
    let (old, _) = delegation(&state, &lead.id, &spec.id, "Ancient", "completed", Some("old")).await;
    set_time(&state, "delegations", "ended_at", &old, now - Duration::days(8)).await;
    let (two_days, _) = delegation(&state, &lead.id, &spec.id, "Two days ago", "completed", Some("ok")).await;
    set_time(&state, "delegations", "ended_at", &two_days, now - Duration::days(2)).await;
    let routine = state
        .db
        .create_routine(&NewRoutine {
            kwant_id: lead.id.clone(),
            channel_id: None,
            name: "Morning briefing".into(),
            instructions: "Summarise".into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            enabled: false,
            next_run_at: None,
            thread_id: None,
            context_message_id: None,
        })
        .await
        .unwrap();
    let ok_run = state.db.start_routine_run(&routine.id, "manual", "running", None).await.unwrap();
    let briefing = state.db.insert_message(&reply(&lead.id, "3 new orders, 1 refund.")).await.unwrap();
    state.db.finish_routine_run(&ok_run.id, "ok", None, Some(&briefing.id)).await.unwrap();

    // Failed: failed and interrupted delegations, a failed run (error carries a key), stuck work
    let (failed, _) = delegation(&state, &lead.id, &spec.id, "Deploy", "failed", Some(&format!("401 invalid key {SECRET}"))).await;
    let (interrupted, _) = delegation(&state, &lead.id, &spec.id, "Audit", "interrupted", Some("kwnt stopped before the task finished")).await;
    let err_run = state
        .db
        .start_routine_run(&routine.id, "schedule", "error", Some(&format!("provider said: {{\"key\":\"{SECRET}\"}}")))
        .await
        .unwrap();
    let (stuck, stuck_thread) = delegation(&state, &lead.id, &spec.id, "Long crawl", "running", None).await;
    set_time(&state, "delegations", "started_at", &stuck, now - Duration::minutes(31)).await;
    let (busy, _) = delegation(&state, &lead.id, &spec.id, "Short crawl", "running", None).await;
    set_time(&state, "delegations", "started_at", &busy, now - Duration::minutes(29)).await;

    let v = get(&app, "/api/away").await;
    let body = v.to_string();
    assert!(!body.contains(SECRET) && !body.contains("hidden-value") && !body.contains("hunter2"), "secret leaked: {body}");

    // The approval carries the live event's fields, masked like the rest of the feed
    let ai0 = find(&v["waiting"], &live.id);
    assert_eq!(ai0["tool_name"], "mcp__mail__send");
    assert_eq!(ai0["details"]["server"], "mail");
    assert_eq!(ai0["details"]["arguments"]["api_key"], "[redacted]");
    assert_eq!(ai0["details"]["arguments"]["to"], "me@example.com");
    assert_eq!(ai0["details"]["arguments"]["command"], "mysql -u bob password=[redacted]");

    // Waiting ignores since; the stale ticket and the answered question aren't there
    assert_eq!(ids(&v["waiting"]), vec![live.id.clone(), q.id.clone()]);
    let qi = find(&v["waiting"], &q.id);
    assert_eq!(qi["kind"], "question");
    assert_eq!(qi["kwant_name"], spec.name.as_str());
    assert_eq!(qi["title"], "Ship tonight?");
    assert_eq!(qi["summary"], "Choices: Yes · No");
    assert_eq!(qi["choices"], json!(["Yes", "No"]));
    assert_eq!(qi["link"], json!({"thread_id": null, "channel_id": chan.id, "direct_kwant_id": null}));
    let ai = find(&v["waiting"], &live.id);
    assert_eq!(ai["kind"], "approval");
    assert_eq!(ai["status"], "pending");
    assert_eq!(ai["title"], "Use send (mail)");
    assert!(ai["summary"].as_str().unwrap().contains("to: me@example.com"));
    // A free-text password in an ordinary argument keeps its key and loses its value
    assert!(ai["summary"].as_str().unwrap().contains("command: mysql -u bob password=[redacted]"), "{}", ai["summary"]);
    // No live turn to follow: it opens the agent's direct chat
    assert_eq!(ai["link"]["direct_kwant_id"], lead.id.as_str());
    assert!(!ids(&v["waiting"]).contains(&stale.id));

    // Finished within the default 7 days
    assert_eq!(ids(&v["finished"]), vec![ok_run.id.clone(), done.clone(), two_days.clone()]);
    let di = find(&v["finished"], &done);
    assert_eq!(di["kind"], "delegation");
    assert_eq!(di["kwant_id"], spec.id.as_str());
    assert_eq!(di["status"], "completed");
    assert_eq!(di["summary"], "Brief");
    assert_eq!(di["link"]["thread_id"], done_thread.as_str());
    let ri = find(&v["finished"], &ok_run.id);
    assert_eq!(ri["kind"], "routine_run");
    assert_eq!(ri["title"], "Morning briefing");
    assert_eq!(ri["summary"], "3 new orders, 1 refund.");
    assert_eq!(ri["link"], json!({"thread_id": null, "channel_id": null, "direct_kwant_id": lead.id}));

    // Failed, plus stuck (31 min) but not busy (29 min)
    let failed_ids = ids(&v["failed"]);
    for id in [&failed, &interrupted, &err_run.id, &stuck] {
        assert!(failed_ids.contains(id), "{id} not failed: {failed_ids:?}");
    }
    assert!(!failed_ids.contains(&busy));
    assert_eq!(failed_ids.len(), 4);
    assert_eq!(find(&v["failed"], &stuck)["status"], "stuck");
    assert_eq!(find(&v["failed"], &stuck)["link"]["thread_id"], stuck_thread.as_str());
    assert_eq!(find(&v["failed"], &interrupted)["status"], "interrupted");
    let fe = find(&v["failed"], &failed)["summary"].as_str().unwrap().to_string();
    assert!(fe.starts_with("401 invalid key") && fe.contains("[redacted]"), "{fe}");
    assert_eq!(find(&v["failed"], &err_run.id)["status"], "error");
    for list in ["waiting", "finished", "failed"] {
        assert_newest_first(&v[list]);
    }
    assert_eq!(v["unseen"], json!({"waiting": 2, "finished": 3, "failed": 4, "total": 9}));
    assert_eq!(v["stuck_after_minutes"], 30);

    // An explicit since narrows finished and failed, but not waiting or stuck
    let v = get(&app, &format!("/api/away?since={}", rfc(now - Duration::days(1)))).await;
    assert_eq!(ids(&v["finished"]), vec![ok_run.id.clone(), done.clone()]);
    assert_eq!(v["waiting"].as_array().unwrap().len(), 2);
    let v = get(&app, &format!("/api/away?since={}", rfc(now + Duration::minutes(5)))).await;
    assert_eq!(v["finished"], json!([]));
    assert_eq!(ids(&v["failed"]), vec![stuck.clone()]);
    let v = get(&app, &format!("/api/away?since={}", rfc(now - Duration::days(9)).replace('+', "%2B"))).await;
    assert!(ids(&v["finished"]).contains(&old));
    // The badge still counts from the default window, whatever since asked for
    assert_eq!(v["unseen"]["finished"], 3);
    for bad in ["yesterday", "2026-10-17", "2026-13-01T00:00:00Z"] {
        let (s, e) = call(&app, "GET", &format!("/api/away?since={bad}"), &[]).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
        assert!(e["error"].as_str().unwrap().contains("RFC 3339"));
    }

    // Seen: the marker becomes the default since and the badge resets (waiting and stuck remain)
    let (s, seen) = call(&app, "POST", "/api/away/seen", &[]).await;
    assert_eq!(s, StatusCode::OK);
    let v = get(&app, "/api/away").await;
    assert_eq!(v["last_seen"], seen["last_seen"]);
    assert_eq!(v["since"], seen["last_seen"]);
    assert_eq!(v["finished"], json!([]));
    assert_eq!(v["unseen"], json!({"waiting": 2, "finished": 0, "failed": 1, "total": 3}));
    // Not a settings key, and not writable through /api/settings
    let (s, _) = call(&app, "GET", "/api/settings", &[]).await;
    assert_eq!(s, StatusCode::OK);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/settings")
                .header("content-type", "application/json")
                .body(Body::from(json!({"away_last_seen": "2000-01-01T00:00:00Z"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(get(&app, "/api/away").await["last_seen"], seen["last_seen"]);

    // Something finishing after the marker shows up again
    let (fresh, _) = delegation(&state, &lead.id, &spec.id, "Fresh", "completed", Some("new")).await;
    let v = get(&app, "/api/away").await;
    assert_eq!(ids(&v["finished"]), vec![fresh]);
    assert_eq!(v["unseen"]["finished"], 1);

    // Same auth as the rest of the API
    for (method, uri) in [("GET", "/api/away"), ("POST", "/api/away/seen")] {
        let (s, _) = call(&app, method, uri, &[("host", "kwnt.example.com")]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn away_lists_are_capped_at_50_newest_first() {
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let (lead, spec) = (&kwants[0].id, &kwants[1].id);
    let now = Utc::now();
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for i in 0..60 {
        let (d, _) = delegation(&state, lead, spec, &format!("done {i}"), "completed", Some("ok")).await;
        set_time(&state, "delegations", "ended_at", &d, now - Duration::minutes(60 - i)).await;
        done.push(d);
        let (f, _) = delegation(&state, lead, spec, &format!("failed {i}"), "failed", Some("boom")).await;
        set_time(&state, "delegations", "ended_at", &f, now - Duration::minutes(60 - i)).await;
        failed.push(f);
    }
    let v = get(&app, "/api/away").await;
    let newest = |all: &[String]| all.iter().rev().take(50).cloned().collect::<Vec<_>>();
    assert_eq!(ids(&v["finished"]), newest(&done));
    assert_eq!(ids(&v["failed"]), newest(&failed));
    // The badge counts everything, not just what fits
    assert_eq!(v["unseen"]["finished"], 60);
    assert_eq!(v["unseen"]["failed"], 60);
}

/// Asks for a tool approval the way claude's permission prompt does, from a live turn in `place`
/// (`(context_id, thread_id)`), and returns the waiting item once it shows up.
async fn live_approval(app: &Router, state: &AppState, kwant: &kwnt::db::models::Kwant, place: (&str, Option<&str>), tool: &str, input: Value) -> Value {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let folder = tempdir().unwrap();
    let (_, token) = mcp_runtime::prepare_turn_with_self(state, kwant, Some(turn_id.clone()), Some(folder.path().to_path_buf()), None).await.unwrap();
    state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: kwant.id.clone(),
        context_id: place.0.into(),
        thread_id: place.1.map(Into::into),
        state: "running".into(),
        since: Utc::now(),
    });
    let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
    let req = Request::post(format!("/api/mcp/permission/{token}"))
        .header("content-type", "application/json")
        .body(Body::from(ask.to_string()))
        .unwrap();
    let mut events = state.event_sender.subscribe();
    let prompt = tokio::spawn(app.clone().oneshot(req));

    let mut item = None;
    for _ in 0..200 {
        let v = get(app, "/api/away").await;
        if let Some(a) = v["waiting"].as_array().unwrap().iter().find(|i| i["kind"] == "approval") {
            item = Some(a.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let item = item.expect("the approval never showed up under waiting");
    assert_eq!(state.db.get_approval(item["id"].as_str().unwrap()).await.unwrap().unwrap().turn_id, turn_id);
    // The item carries what the live event did, so the client can render it the same way.
    let (tool_name, details) = loop {
        match events.try_recv() {
            Ok(kwnt::drivers::events::RuntimeEvent::ApprovalRequired { id, tool_name, details }) if id == item["id"] => break (tool_name, details),
            Ok(_) => continue,
            Err(e) => panic!("no approval_required event for the item: {e:?}"),
        }
    };
    assert_eq!(item["tool_name"], tool_name);
    assert_eq!(tool_name, tool);
    assert_eq!(item["details"], details);
    assert!(details.is_object() && details != json!({}), "{details}");

    assert!(state.broker.resolve(item["id"].as_str().unwrap(), ApprovalDecision::AllowOnce).await);
    let resp = prompt.await.unwrap().unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains(r#"\"behavior\":\"allow\""#), "{}", String::from_utf8_lossy(&bytes));
    mcp_runtime::finish_turn(state, &token).await;
    state.turns.end_turn(&turn_id);
    if item["details"]["source"] == "claude_builtin" {
        let real = folder.path().canonicalize().unwrap();
        assert_eq!(item["details"]["cwd"], real.display().to_string(), "{item}");
    }
    item
}

async fn live_approval_link(app: &Router, state: &AppState, kwant: &kwnt::db::models::Kwant, place: (&str, Option<&str>)) -> Value {
    live_approval(app, state, kwant, place, "mcp__echo__echo", json!({"text": "hi"})).await["link"].clone()
}

#[tokio::test]
async fn live_approvals_link_to_the_chat_their_turn_runs_in() {
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let (lead, spec) = (kwants[0].clone(), kwants[1].clone());
    let echo = state
        .db
        .create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &McpServer {
            id: String::new(),
            name: "echo".into(),
            transport: "stdio".into(),
            command: Some("/usr/bin/true".into()),
            args: vec![],
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            enabled: true,
            timeout_secs: 30,
            created_at: None,
        })
        .await
        .unwrap();
    state.db.set_kwant_mcp_servers(&lead.id, std::slice::from_ref(&echo.id)).await.unwrap();
    let chan = state
        .db
        .create_channel(&kwnt::db::spaces::SpaceId::default_space(), &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), spec.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();
    let thread = state.db.create_thread(&lead.id, "Side quest", "chat", None, "user").await.unwrap();

    let link = live_approval_link(&app, &state, &lead, (&chan.id, None)).await;
    assert_eq!(link, json!({"thread_id": null, "channel_id": chan.id, "direct_kwant_id": null}));
    let link = live_approval_link(&app, &state, &lead, (&lead.id, Some(&thread.id))).await;
    assert_eq!(link, json!({"thread_id": thread.id, "channel_id": null, "direct_kwant_id": null}));
    let link = live_approval_link(&app, &state, &lead, (&lead.id, None)).await;
    assert_eq!(link, json!({"thread_id": null, "channel_id": null, "direct_kwant_id": lead.id}));

    // A claude built-in Bash request reloads with what the built-in card draws: command, folder, outside_folder
    let bash = live_approval(&app, &state, &lead, (&chan.id, None), "Bash", json!({"command": "touch x", "description": "Create x"})).await;
    assert_eq!(bash["tool_name"], "Bash");
    assert_eq!(bash["details"]["source"], "claude_builtin");
    assert_eq!(bash["details"]["arguments"]["command"], "touch x");
    assert!(bash["details"]["cwd"].as_str().is_some_and(|c| !c.is_empty()), "{bash}"); // and equals the turn's folder (checked in live_approval)
    assert_eq!(bash["details"]["outside_folder"], false);
    assert_eq!(bash["details"]["kwant_id"], lead.id);
    assert_eq!(bash["link"], json!({"thread_id": null, "channel_id": chan.id, "direct_kwant_id": null}));
}
