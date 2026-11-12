//! 0.9.5 B: Inbox "Done while you were away" (release-095/inbox-away/brief-b.md). Done items stay
//! for 7 days whatever was seen, carry their answer, trail and why, and a lead's `Done:` is a
//! team job.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::{ApprovalRequest, PermissionBroker},
    config::Config,
    db::{
        continuity::NewRoutine,
        missions::{NewTask, TaskChange},
        models::{Kwant, NewApproval, NewChannel, NewMessage},
        spaces::SpaceId,
        threads::NewDelegation,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    (create_router(state.clone()), state, tmp)
}

async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn inbox(app: &Router) -> Value {
    let (s, v) = call(app, "GET", "/api/inbox").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn find<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    v["items"].as_array().unwrap().iter().find(|i| i["id"] == id)
}

/// Sets a timestamp column the way kwnt writes it (sqlx's encoding), so comparisons line up.
async fn set_time(state: &AppState, table: &str, column: &str, id: &str, t: DateTime<Utc>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", state.config.data_dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query(&format!("UPDATE {table} SET {column} = ? WHERE id = ?")).bind(t).bind(id).execute(&pool).await.unwrap();
    pool.close().await;
}

fn message(role: &str, sender: &str, direct: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: role.into(),
        sender_id: sender.into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    }
}

fn step(tool: &str, title: &str, at: &str) -> Value {
    json!({"kind": "tool", "status": "ok", "tool": tool, "title": title, "started_at": at, "ended_at": at})
}

/// A reply of `k` in its direct chat with `steps` saved as its activity, as turns save them.
async fn reply(state: &AppState, k: &Kwant, text: &str, steps: Vec<Value>, extra: Value) -> String {
    let m = state.db.insert_message(&message("kwant", &k.name, &k.id, text)).await.unwrap();
    let mut meta = json!({"activity": {"steps": steps}});
    if let Value::Object(o) = extra {
        for (key, v) in o {
            meta[key] = v;
        }
    }
    state.db.set_message_meta(&m.id, &meta).await.unwrap();
    m.id
}

async fn routine_run(state: &AppState, k: &Kwant, name: &str, instructions: &str, answer: &str, steps: Vec<Value>) -> (String, String) {
    let routine = state
        .db
        .create_routine(&NewRoutine {
            kwant_id: k.id.clone(),
            channel_id: None,
            name: name.into(),
            instructions: instructions.into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            enabled: false,
            next_run_at: None,
            thread_id: None,
            context_message_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let run = state.db.start_routine_run(&routine.id, "manual", "running", None).await.unwrap();
    let msg = reply(state, k, answer, steps, json!({})).await;
    state.db.finish_routine_run(&run.id, "ok", None, Some(&msg)).await.unwrap();
    (run.id, msg)
}

#[tokio::test]
async fn a_finished_run_stays_after_seen_with_its_answer_trail_and_why_until_seven_days() {
    let (app, state, _tmp) = setup().await;
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let at = "2026-11-12T09:00:00.000Z";
    let read_only = vec![
        json!({"kind": "thinking", "status": "ok", "title": "Thinking", "started_at": at}),
        step("WebFetch", "Opening booktun.ing", at),
        step("Read", "Reading status.md", at),
    ];
    let (run, msg) = routine_run(&state, &k, "Uptime", "Check booktun.ing is up.\nPost one line.", "## Status\n**booktun.ing is up**, 200 in 180 ms.", read_only).await;
    let writes = vec![step("Bash", "Running `deploy.sh`", at)];
    let (wrote, _) = routine_run(&state, &k, "Deploy", "Deploy the site.", "Deployed.", writes).await;

    let v = inbox(&app).await;
    let item = find(&v, &run).unwrap_or_else(|| panic!("{run} missing: {v}"));
    assert_eq!(item["group"], "finished");
    assert_eq!(item["unread"], true);
    assert_eq!(item["result_message_id"], json!(msg));
    assert_eq!(item["result_conversation_id"], json!(k.id));
    assert_eq!(item["result_link"]["direct_kwant_id"], json!(k.id));
    assert_eq!(item["answer_excerpt"], "Status\nbooktun.ing is up, 200 in 180 ms.");
    assert_eq!(item["why"], "Check booktun.ing is up.\nPost one line.");
    assert_eq!(item["why_kind"], "routine");
    assert_eq!(
        item["trail"],
        json!([
            {"at": at, "kind": "page", "label": "Opening booktun.ing", "detail": null, "status": "ok", "by": null},
            {"at": at, "kind": "tool", "label": "Reading status.md", "detail": null, "status": "ok", "by": null},
        ])
    );
    assert_eq!(item["trail_dropped"], 0);
    assert_eq!(item["result_thread_id"], Value::Null);
    assert_eq!(item["source"], json!({"kind": "routine", "routine_id": item["source"]["routine_id"], "run_id": run, "trigger": "manual"}));
    assert!(item["source"]["routine_id"].is_string());
    // A job that only read changed nothing; one that ran a command may have
    assert_eq!((item["changed"].clone(), item["changed_nothing"].clone()), (json!(false), json!(true)));
    let w = find(&v, &wrote).unwrap();
    assert_eq!((w["changed"].clone(), w["changed_nothing"].clone()), (json!(true), json!(false)));
    assert_eq!(w["trail"][0]["kind"], "write");
    assert_eq!(v["counts"]["badge"], 2, "{v}");

    // Seen only clears unread: both stay, and the badge counts unread only
    let (s, _) = call(&app, "POST", "/api/away/seen").await;
    assert_eq!(s, StatusCode::OK);
    let v = inbox(&app).await;
    let item = find(&v, &run).unwrap_or_else(|| panic!("{run} cleared by seen: {v}"));
    assert_eq!(item["unread"], false);
    assert_eq!(item["answer_excerpt"], "Status\nbooktun.ing is up, 200 in 180 ms.");
    assert_eq!(find(&v, &wrote).unwrap()["unread"], false);
    assert_eq!(v["counts"]["badge"], 0, "{v}");
    assert_eq!(v["counts"]["finished"], 2);
    // An older since is cut to the 7 days
    let (s, old) = call(&app, "GET", "/api/inbox?since=2020-01-01T00:00:00Z").await;
    assert_eq!(s, StatusCode::OK);
    let floor = DateTime::parse_from_rfc3339(old["since"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!(floor > Utc::now() - Duration::days(7) - Duration::minutes(1), "{old}");

    // Gone once it ended more than 7 days ago
    set_time(&state, "routine_runs", "ended_at", &run, Utc::now() - Duration::days(7) - Duration::minutes(5)).await;
    let v = inbox(&app).await;
    assert!(find(&v, &run).is_none(), "{v}");
    assert!(find(&v, &wrote).is_some());
}

#[tokio::test]
async fn a_leads_done_line_is_a_team_job_with_members_trail_and_why() {
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let (lead, member) = (kwants[0].clone(), kwants[1].clone());
    let now = Utc::now();

    // The person's request in the lead's chat, then a hand-off from that chat
    let ask = state.db.insert_message(&message("user", "user", &lead.id, "Find three book bloggers\nwho review fantasy")).await.unwrap();
    set_time(&state, "messages", "created_at", &ask.id, now - Duration::minutes(10)).await;
    let (_, d) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: lead.id.clone(),
            specialist_kwant_id: member.id.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(lead.id.clone()),
            origin_thread_id: None,
            team_id: None,
            task: "List fantasy book bloggers".into(),
            title: "Bloggers".into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    set_time(&state, "delegations", "created_at", &d.id, now - Duration::minutes(8)).await;
    assert!(state.db.mark_delegation_running(&d.id).await.unwrap());
    let t1 = "2026-11-12T09:01:00.000Z";
    let found = reply(&state, &member, "Three bloggers: A, B, C.", vec![step("mcp__kwnt__browser_open", "Opening fantasyreads.example", t1)], json!({})).await;
    assert!(state.db.finish_delegation(&d.id, "completed", None, Some(&found)).await.unwrap());

    // The lead's last reply, which keep-going marked with its Done: line
    let t2 = "2026-11-12T09:02:00.000Z";
    let done = reply(
        &state,
        &lead,
        "Here are three fantasy book bloggers.\n\nDone: three bloggers listed with links.",
        vec![step("Read", "Reading the list", t2)],
        json!({"chain_done": {"text": "three bloggers listed with links.", "at": Utc::now()}}),
    )
    .await;
    // A plain reply is not a team job
    let plain = reply(&state, &lead, "Hello", vec![], json!({})).await;

    let v = inbox(&app).await;
    let job = find(&v, &done).unwrap_or_else(|| panic!("no team job: {v}"));
    assert!(find(&v, &plain).is_none());
    assert_eq!(job["kind"], "team_job");
    assert_eq!(job["group"], "finished");
    assert_eq!(job["lead"], json!({"kwant_id": lead.id, "kwant_name": lead.name}));
    assert_eq!(job["kwant_name"], json!(lead.name));
    assert_eq!(job["members"], json!([{"kwant_id": member.id, "kwant_name": member.name}]));
    assert_eq!(job["source"], json!({"kind": "team_job", "delegation_id": d.id}));
    assert_eq!(job["status"], "completed");
    assert_eq!(job["done_line"], "three bloggers listed with links.");
    assert_eq!(job["title"], "Find three book bloggers");
    assert_eq!(job["why"], "Find three book bloggers");
    assert_eq!(job["why_kind"], "request");
    assert_eq!(job["result_message_id"], json!(done));
    assert_eq!(job["result_conversation_id"], json!(lead.id));
    assert!(job["answer_excerpt"].as_str().unwrap().starts_with("Here are three fantasy book bloggers."));
    // The members' steps join the lead's, in time order, with who did them
    let trail = job["trail"].as_array().unwrap();
    assert_eq!(trail.len(), 2, "{job}");
    assert_eq!((trail[0]["kind"].as_str(), trail[0]["by"].clone()), (Some("page"), json!(member.name)));
    assert_eq!((trail[1]["label"].as_str(), trail[1]["by"].clone()), (Some("Reading the list"), Value::Null));
    assert_eq!(job["changed_nothing"], true, "opening a page and reading changed nothing");
    assert_eq!(job["unread"], true);
    // Team jobs carry unread but are not in the badge: only the member's finished hand-off is,
    // as on /api/away
    assert_eq!(v["counts"]["badge"], 1, "{v}");
    let (_, away) = call(&app, "GET", "/api/away").await;
    assert_eq!(v["counts"]["badge"], away["unseen"]["total"], "{away}");

    // Seen: still listed, now read
    call(&app, "POST", "/api/away/seen").await;
    let v = inbox(&app).await;
    assert_eq!(find(&v, &done).unwrap()["unread"], false, "{v}");
    assert_eq!(v["counts"]["badge"], 0, "{v}");
}

#[tokio::test]
async fn an_approval_is_titled_with_its_own_question() {
    let (app, state, _tmp) = setup().await;
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let ask = |tool: &str, args: Value| NewApproval {
        turn_id: "turn-gone".into(),
        kwant_id: k.id.clone(),
        tool_name: format!("mcp__mail__{tool}"),
        parameters: json!({"server": "mail", "tool": tool, "arguments": args}),
    };
    let (mut ids, mut waiting) = (Vec::new(), Vec::new());
    for (tool, args) in [("send", json!({"to": "list@example.com", "subject": "Spring sale: 15% off"})), ("delete", json!({"id": 42}))] {
        let a = state.db.save_approval(&ask(tool, args)).await.unwrap();
        // The broker keeps it pending while its receiver lives
        waiting.push(state.broker.request_approval(ApprovalRequest { id: a.id.clone(), kwant_id: k.id.clone(), tool_name: a.tool_name.clone(), details: json!({}) }).await);
        ids.push(a.id);
    }
    let v = inbox(&app).await;
    let send = find(&v, &ids[0]).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(send["title"], "Approve “Spring sale: 15% off”?");
    assert_eq!(send["question"], "Approve “Spring sale: 15% off”?");
    assert_eq!(send["tool_name"], "mcp__mail__send", "the card still knows its tool");
    assert_eq!(send["unread"], true);
    // Nothing named: no question, and today's tool title
    let del = find(&v, &ids[1]).unwrap();
    assert!(del.get("question").is_none(), "{del}");
    assert_eq!(del["title"], "Use delete (mail)");
    // /api/away keeps today's title
    let (_, away) = call(&app, "GET", "/api/away").await;
    let a = away["waiting"].as_array().unwrap().iter().find(|i| i["id"] == json!(ids[0])).unwrap();
    assert_eq!(a["title"], "Use send (mail)");
}

/// A lead's `Done:` reply in its direct chat, as keep-going marks it.
async fn done_reply(state: &AppState, lead: &Kwant, text: &str, steps: Vec<Value>) -> String {
    reply(state, lead, text, steps, json!({"chain_done": {"text": text, "at": Utc::now()}})).await
}

#[tokio::test]
async fn the_inbox_badge_matches_away_without_team_jobs() {
    let (app, state, _tmp) = setup().await;
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let at = "2026-11-12T09:00:00.000Z";
    let (run, _) = routine_run(&state, &k, "Uptime", "Check the site.", "Up.", vec![step("WebFetch", "Opening booktun.ing", at)]).await;
    let job = done_reply(&state, &k, "Done: three bloggers listed.", vec![step("GMAIL_SEND_EMAIL", "Sending the list", at)]).await;

    let v = inbox(&app).await;
    assert_eq!(find(&v, &run).unwrap()["unread"], true);
    let j = find(&v, &job).unwrap_or_else(|| panic!("no team job: {v}"));
    assert_eq!(j["unread"], true, "a team job still says it is unread");
    assert_eq!(j["trail"][0]["kind"], "message", "sending mail is a message step");
    assert_eq!(j["changed"], true, "a message counts as changed");
    let (_, away) = call(&app, "GET", "/api/away").await;
    assert_eq!(away["unseen"]["total"], 1, "{away}");
    assert_eq!(v["counts"]["badge"], 1, "the routine run only: {v}");
    assert_eq!(v["counts"]["badge"], away["unseen"]["total"]);
}

/// Timing for impl-b.md: 50 finished and 50 failed done items with a 30-step activity each, 10
/// reviews and 5 team jobs; prints the median of 5 `GET /api/inbox`. Run by hand:
/// `cargo test --test inbox_away_b_test inbox_timing -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn inbox_timing() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let (lead, member) = (kwants[0].clone(), kwants[1].clone());
    let steps = |n: usize| -> Vec<Value> {
        (0..30)
            .map(|i| match i % 3 {
                0 => step("WebFetch", &format!("Opening page {n}.{i}"), "2026-11-12T09:00:00.000Z"),
                1 => step("Read", &format!("Reading file {n}.{i}"), "2026-11-12T09:00:01.000Z"),
                _ => step("Bash", &format!("Running step {n}.{i}"), "2026-11-12T09:00:02.000Z"),
            })
            .collect()
    };
    for n in 0..50 {
        routine_run(&state, &k_of(&kwants, n), &format!("Finished {n}"), "Check the site.\nPost one line.", "## Status\nAll good.", steps(n)).await;
    }
    for n in 0..50 {
        let k = k_of(&kwants, n);
        let routine = state
            .db
            .create_routine(&NewRoutine {
                kwant_id: k.id.clone(),
                name: format!("Failed {n}"),
                instructions: "Deploy the site.".into(),
                schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        let run = state.db.start_routine_run(&routine.id, "manual", "running", None).await.unwrap();
        let msg = reply(&state, &k, "Deploy failed: 401.", steps(100 + n), json!({})).await;
        state.db.finish_routine_run(&run.id, "error", Some("401 from the host"), Some(&msg)).await.unwrap();
    }
    let chan = state
        .db
        .create_channel(&space, &NewChannel {
            name: "ops".into(),
            description: None,
            lead_kwant_id: lead.id.clone(),
            member_kwants: vec![lead.id.clone(), member.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();
    let change = |r: &str| TaskChange { kind: "state".into(), reason: Some(r.into()), ..Default::default() };
    for n in 0..10 {
        let t = state
            .db
            .create_task(&space, &NewTask {
                title: format!("Review {n}"),
                acceptance: "- Works\n- No new dependencies".into(),
                state: "ready".into(),
                owner_kwant_id: Some(member.id.clone()),
                max_attempts: 10,
                source_kind: "person".into(),
                channel_id: Some(chan.id.clone()),
                created_by: "person".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        state.db.claim_task(&t.id, &member.id, None, None, 600_000, false, &member.id).await.unwrap().unwrap();
        assert!(state.db.move_task(&t.id, "working", "review", &member.id, &change("done")).await.unwrap());
    }
    for n in 0..5 {
        state.db.insert_message(&message("user", "user", &lead.id, &format!("Job {n}: find bloggers"))).await.unwrap();
        let (_, d) = state
            .db
            .create_delegation(&NewDelegation {
                lead_kwant_id: lead.id.clone(),
                specialist_kwant_id: member.id.clone(),
                origin_channel_id: None,
                origin_kwant_id: Some(lead.id.clone()),
                origin_thread_id: None,
                team_id: None,
                task: format!("Part of job {n}"),
                title: format!("Job {n}"),
                continue_lead: false,
                depth: 1,
                follow_up_at: None,
            })
            .await
            .unwrap();
        assert!(state.db.mark_delegation_running(&d.id).await.unwrap());
        let found = reply(&state, &member, "Found them.", steps(200 + n), json!({})).await;
        assert!(state.db.finish_delegation(&d.id, "completed", None, Some(&found)).await.unwrap());
        done_reply(&state, &lead, &format!("Done: job {n} finished."), steps(300 + n)).await;
    }

    let v = inbox(&app).await;
    let c = &v["counts"];
    println!("items {} counts {c}", v["items"].as_array().unwrap().len());
    let mut ms = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let v = inbox(&app).await;
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(v["items"].as_array().unwrap().len() >= 100);
    }
    ms.sort_by(f64::total_cmp);
    let profile = if cfg!(debug_assertions) { "debug (test profile)" } else { "release" };
    println!("GET /api/inbox: median {:.1} ms of 5 {ms:?}, {profile}", ms[2]);
}

fn k_of(kwants: &[Kwant], n: usize) -> Kwant {
    kwants[n % kwants.len()].clone()
}
