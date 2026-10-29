//! The Swarm Auditor (Phase D2): which tasks get audited, who challenges, the caps, the two tool
//! calls, and that an audit never verifies, fails or moves a task. Off by default.
//! Contract: docs/mission-control.md §18.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        missions::{NewCheck, NewMission, NewTask, Task},
        models::{Kwant, NewKwant},
        performance::NewObservation,
        spaces::{SpaceId, DEFAULT_SPACE_ID},
        threads::NewDelegation,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::auditor,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
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
    let space = SpaceId::default_space();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app: create_router(state.clone()), state, general, _tmp: tmp };
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"mission_board": true}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    e
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

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = call(&e.app, method, uri, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn auditor_on(e: &Env, cap: i64) {
    person(e, "POST", "/api/settings", Some(json!({"auditor": "on", "auditor_daily_cap": cap}))).await;
}

/// A bot of the default space that never runs a turn here (the tests that open an audit don't
/// drive it).
async fn bot(e: &Env, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Tester".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    e.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn task_in(e: &Env, space: &str, owner: Option<&Kwant>, priority: i64, state: &str) -> Task {
    let t = NewTask {
        title: "Ship the export".into(),
        acceptance: "The export file opens".into(),
        state: state.into(),
        priority,
        owner_kwant_id: owner.map(|k| k.id.clone()),
        budget_tokens: 0,
        max_attempts: 3,
        source_kind: "person".into(),
        created_by: "person".into(),
        ..Default::default()
    };
    e.state.db.create_task(&SpaceId::new(space), &t).await.unwrap()
}

async fn task(e: &Env, owner: Option<&Kwant>, priority: i64, state: &str) -> Task {
    task_in(e, DEFAULT_SPACE_ID, owner, priority, state).await
}

/// A check the person added (it counts, and isn't the owner's).
async fn person_check(e: &Env, t: &Task) {
    e.state
        .db
        .add_check(&NewCheck {
            task_id: t.id.clone(),
            kind: "file_exists".into(),
            spec: json!({"path": "out.csv"}),
            required: true,
            approved: true,
            requested_by: "person".into(),
            approved_by: Some("person".into()),
        })
        .await
        .unwrap();
}

/// `bot` verified `n` tasks of class `general` (Performance Market rows).
async fn verified(e: &Env, bot: &Kwant, n: usize) {
    for i in 0..n {
        let o = NewObservation {
            task_id: format!("obs-{}-{i}", bot.id),
            space_id: bot.space_id.clone(),
            mission_id: None,
            task_class: "general".into(),
            kwant_id: bot.id.clone(),
            engine: "cli".into(),
            model: "m".into(),
            started_at: None,
            finished_at: chrono::Utc::now(),
            latency_ms: 0,
            runs: 1,
            fresh_tokens: 0,
            cost_usd: None,
            result: "verified",
            approvals: 0,
            check_approvals: 0,
            unblocks: 0,
            retries: 0,
            autofix_attempts: 0,
        };
        e.state.db.store_observation(&o, "live").await.unwrap();
    }
}

/// A turn token for `bot` with its own tools.
async fn token(e: &Env, bot: &Kwant) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id), None, Some(scope)).await.unwrap().1
}

async fn tool(e: &Env, bot: &Kwant, name: &str, args: Value) -> (Value, bool, String) {
    let t = token(e, bot).await;
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(&t), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    (serde_json::from_str(&text).unwrap_or(Value::Null), v["result"]["isError"].as_bool().unwrap_or(false), text)
}

async fn events(e: &Env, tid: &str) -> Vec<String> {
    e.state.db.task_events(tid, 200).await.unwrap().into_iter().map(|ev| ev.kind).collect()
}

/// Task state and checks, which an audit must never change.
async fn verification(e: &Env, tid: &str) -> (String, Vec<(String, String)>) {
    let t = e.state.db.get_task(tid).await.unwrap().unwrap();
    let checks = e.state.db.list_checks(tid).await.unwrap().into_iter().map(|c| (c.id, c.status)).collect();
    (t.state, checks)
}

/// An audit of `t` waiting for its challenge, opened without running any turn.
async fn open_audit(e: &Env, t: &Task) -> kwnt::db::auditor::Audit {
    let a = auditor::start(&e.state, t, "manual").await.unwrap();
    assert!(e.state.db.move_audit(&a.id, "queued", "challenge", None).await.unwrap());
    a
}

#[tokio::test]
async fn the_auditor_is_off_by_default() {
    let e = setup().await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["auditor"].as_str(), s["auditor_daily_cap"].as_i64()), (Some("off"), Some(5)), "{s}");
    let (owner, other) = (bot(&e, "Owner").await, bot(&e, "Other").await);

    // A high-impact task reaching review gets no audit while it is off
    let t = task(&e, Some(&owner), 3, "ready").await;
    let claim = tool(&e, &owner, "task_claim", json!({"task_id": t.id})).await;
    assert!(!claim.1, "{}", claim.2);
    let moved = tool(&e, &owner, "task_update", json!({"task_id": t.id, "status": "review"})).await;
    assert!(!moved.1, "{}", moved.2);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(e.state.db.audits_of(&t.id).await.unwrap().is_empty());
    assert!(!events(&e, &t.id).await.iter().any(|k| k.starts_with("audit")));

    // The person can't start one, and the tools refuse
    let (status, v) = call(&e.app, "POST", &format!("/api/tasks/{}/audit", t.id), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    let a = e.state.db.create_audit(&t.id, DEFAULT_SPACE_ID, 1, "manual", &other.id, &owner.id).await.unwrap().unwrap();
    e.state.db.move_audit(&a.id, "queued", "challenge", None).await.unwrap();
    let (_, err, text) = tool(&e, &other, "audit_challenge", json!({"audit_id": a.id, "risks": []})).await;
    assert!(err && text.contains("off"), "{text}");
    // Reading still works
    let r = person(&e, "GET", &format!("/api/tasks/{}/risks", t.id), None).await;
    assert_eq!(r["audits"].as_array().unwrap().len(), 1, "{r}");
}

#[tokio::test]
async fn high_impact_or_unchecked_tasks_trigger_an_audit() {
    let e = setup().await;
    let owner = bot(&e, "Owner").await;
    let high = task(&e, Some(&owner), 2, "review").await;
    assert_eq!(auditor::trigger_of(&high, &[]), Some("high_impact"));
    person_check(&e, &high).await;
    let checks = e.state.db.list_checks(&high.id).await.unwrap();
    assert_eq!(auditor::trigger_of(&high, &checks), Some("high_impact"), "high impact even with a check");

    let low = task(&e, Some(&owner), 0, "review").await;
    assert_eq!(auditor::trigger_of(&low, &[]), Some("low_confidence"), "nothing outside will check it");
    person_check(&e, &low).await;
    let checks = e.state.db.list_checks(&low.id).await.unwrap();
    assert_eq!(auditor::trigger_of(&low, &checks), None, "a normal task with a check that counts");

    // The owner's own check doesn't count for confidence
    let own = task(&e, Some(&owner), 1, "review").await;
    let mine = e
        .state
        .db
        .add_check(&NewCheck {
            task_id: own.id.clone(),
            kind: "file_exists".into(),
            spec: json!({"path": "x"}),
            required: true,
            approved: true,
            requested_by: owner.id.clone(),
            approved_by: Some("auto".into()),
        })
        .await
        .unwrap();
    assert_eq!(auditor::trigger_of(&own, &[mine]), Some("low_confidence"));

    // Nobody owns it: nobody to answer, no audit
    let orphan = task(&e, None, 3, "review").await;
    assert_eq!(auditor::trigger_of(&orphan, &[]), None);
}

#[tokio::test]
async fn the_challenger_is_never_the_owner_or_its_lead_loop() {
    let e = setup().await;
    let (owner, lead, handoff) = (bot(&e, "Owner").await, bot(&e, "Lead").await, bot(&e, "Handoff").await);
    let (expert, fresh) = (bot(&e, "Expert").await, bot(&e, "Fresh").await);

    let mission = e
        .state
        .db
        .create_mission(
            &SpaceId::default_space(),
            &NewMission {
                title: "M".into(),
                goal: "G".into(),
                status: "active".into(),
                lead_kwant_id: lead.id.clone(),
                workspace_path: e._tmp.path().join("m").to_string_lossy().into(),
                created_by: "person".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (_, d) = e
        .state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: handoff.id.clone(),
            specialist_kwant_id: owner.id.clone(),
            origin_channel_id: Some(e.general.clone()),
            origin_kwant_id: None,
            origin_thread_id: None,
            team_id: None,
            task: "do it".into(),
            title: "do it".into(),
            continue_lead: false,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    let t = e
        .state
        .db
        .create_task(
            &SpaceId::default_space(),
            &NewTask {
                mission_id: Some(mission.id.clone()),
                title: "T".into(),
                state: "review".into(),
                priority: 2,
                owner_kwant_id: Some(owner.id.clone()),
                max_attempts: 3,
                source_kind: "delegation".into(),
                source_id: Some(d.id.clone()),
                created_by: "person".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let out = auditor::excluded(&e.state, &t).await;
    for k in [&owner, &lead, &handoff] {
        assert!(out.contains(&k.id), "{} must be excluded: {out:?}", k.name);
    }

    // Most verified in the class wins, whatever the name
    verified(&e, &owner, 9).await;
    verified(&e, &lead, 9).await;
    verified(&e, &handoff, 9).await;
    verified(&e, &expert, 3).await;
    verified(&e, &fresh, 2).await;
    assert_eq!(auditor::pick_challenger(&e.state, &t).await.unwrap().id, expert.id);

    // Equal records: the one that challenged least this week
    verified(&e, &fresh, 3).await;
    let other = task(&e, Some(&owner), 2, "review").await;
    e.state.db.create_audit(&other.id, DEFAULT_SPACE_ID, other.attempts, "manual", &expert.id, &owner.id).await.unwrap().unwrap();
    assert_eq!(auditor::pick_challenger(&e.state, &t).await.unwrap().id, fresh.id);
}

#[tokio::test]
async fn one_audit_per_attempt_and_a_daily_cap() {
    let e = setup().await;
    let (owner, _other) = (bot(&e, "Owner").await, bot(&e, "Other").await);
    auditor_on(&e, 2).await;
    let t = task(&e, Some(&owner), 2, "review").await;
    let a = auditor::start(&e.state, &t, "high_impact").await.unwrap();
    assert_eq!((a.status.as_str(), a.attempt, a.owner_kwant_id.as_str()), ("queued", t.attempts, owner.id.as_str()));
    assert_ne!(a.challenger_kwant_id, owner.id);
    assert!(auditor::start(&e.state, &t, "manual").await.is_err(), "same attempt");
    let (status, v) = call(&e.app, "POST", &format!("/api/tasks/{}/audit", t.id), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(events(&e, &t.id).await.contains(&"audit_started".to_string()));

    // A new attempt can be audited again
    e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET attempts = attempts + 1 WHERE id = '{}'", t.id)).await.unwrap();
    let again = e.state.db.get_task(&t.id).await.unwrap().unwrap();
    auditor::start(&e.state, &again, "high_impact").await.unwrap();

    // Two today: the third is skipped and logged
    let t3 = task(&e, Some(&owner), 2, "review").await;
    let why = auditor::start(&e.state, &t3, "high_impact").await.unwrap_err();
    assert!(why.contains("cap"), "{why}");
    assert!(e.state.db.audits_of(&t3.id).await.unwrap().is_empty());
    assert!(events(&e, &t3.id).await.contains(&"audit_skipped".to_string()));

    // No owner, or nobody else to challenge
    let orphan = task(&e, None, 2, "review").await;
    assert!(auditor::start(&e.state, &orphan, "manual").await.is_err());
}

#[tokio::test]
async fn a_challenge_needs_evidence_and_only_the_challenger_gives_it() {
    let e = setup().await;
    let owner = bot(&e, "Owner").await;
    auditor_on(&e, 5).await;
    let t = task(&e, Some(&owner), 2, "review").await;
    person_check(&e, &t).await;
    let before = verification(&e, &t.id).await;
    let a = open_audit(&e, &t).await;
    let challenger = e.state.db.get_kwant(&a.challenger_kwant_id).await.unwrap().unwrap();

    // The owner can't challenge, nor answer before the challenge
    let (_, err, text) = tool(&e, &owner, "audit_challenge", json!({"audit_id": a.id, "risks": []})).await;
    assert!(err, "{text}");
    let (_, err, text) = tool(&e, &owner, "audit_rebut", json!({"audit_id": a.id, "answers": []})).await;
    assert!(err && text.contains("isn't waiting"), "{text}");

    // Evidence is required, at most 5 risks, evidence ids must be the task's
    let no_ev = json!({"audit_id": a.id, "risks": [{"risk": "The file is empty", "evidence": "  "}]});
    let (_, err, text) = tool(&e, &challenger, "audit_challenge", no_ev).await;
    assert!(err && text.contains("evidence"), "{text}");
    let six: Vec<Value> = (0..6).map(|i| json!({"risk": format!("r{i}"), "evidence": "saw it"})).collect();
    let (_, err, text) = tool(&e, &challenger, "audit_challenge", json!({"audit_id": a.id, "risks": six})).await;
    assert!(err && text.contains("At most 5"), "{text}");
    let foreign = json!({"audit_id": a.id, "risks": [{"risk": "r", "evidence": "e", "evidence_ids": ["nope"]}]});
    let (_, err, text) = tool(&e, &challenger, "audit_challenge", foreign).await;
    assert!(err && text.contains("isn't on this task"), "{text}");

    // Mid-audit, neither side can review, update or add checks to the task
    for (who, name, args) in [
        (&challenger, "task_review", json!({"task_id": t.id, "verdict": "pass", "note": "fine"})),
        (&owner, "task_update", json!({"task_id": t.id, "status": "working"})),
        (&owner, "task_request_check", json!({"task_id": t.id, "kind": "file_exists", "spec": {"path": "y"}})),
    ] {
        let (_, err, text) = tool(&e, who, name, args).await;
        assert!(err && text.contains("during its audit"), "{name}: {text}");
    }

    let two = json!({"audit_id": a.id, "risks": [
        {"risk": "The export drops rows past 10k", "evidence": "Opened out.csv: 10000 rows; the source has 12 431."},
        {"risk": "No check covers the header", "evidence": "The only check is file_exists."},
    ]});
    let (v, err, text) = tool(&e, &challenger, "audit_challenge", two.clone()).await;
    assert!(!err, "{text}");
    let ids: Vec<String> = v["risks"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 2);
    let (_, err, text) = tool(&e, &challenger, "audit_challenge", two).await;
    assert!(err, "one challenge only: {text}");

    // The rebuttal: a dispute needs evidence; only the owner answers, once
    let (_, err, text) = tool(&e, &challenger, "audit_rebut", json!({"audit_id": a.id, "answers": []})).await;
    assert!(err && text.contains("owner"), "{text}");
    let bare = json!({"audit_id": a.id, "answers": [{"risk_id": ids[0], "stance": "disputed", "answer": "no"}]});
    let (_, err, text) = tool(&e, &owner, "audit_rebut", bare).await;
    assert!(err && text.contains("evidence"), "{text}");
    let answers = json!({"audit_id": a.id, "answers": [
        {"risk_id": ids[0], "stance": "disputed", "answer": "The source has 10000 rows", "evidence": "wc -l source.csv: 10001"},
    ]});
    let (v, err, text) = tool(&e, &owner, "audit_rebut", answers.clone()).await;
    assert!(!err, "{text}");
    assert_eq!(v["unresolved"], 2, "a dispute isn't proof; the unanswered one stays open: {v}");
    let (_, err, _) = tool(&e, &owner, "audit_rebut", answers).await;
    assert!(err, "one rebuttal only");

    // The task, its checks and its verification are exactly as before
    assert_eq!(verification(&e, &t.id).await, before);
    let r = person(&e, "GET", &format!("/api/tasks/{}/risks", t.id), None).await;
    assert_eq!(r["open_risks"], 2, "{r}");
    let risks = r["audits"][0]["risks"].as_array().unwrap();
    assert_eq!((risks[0]["status"].as_str(), risks[1]["status"].as_str()), (Some("disputed"), Some("open")));
    assert_eq!(r["audits"][0]["status"], "done");
    let kinds = events(&e, &t.id).await;
    for k in ["audit_started", "audit_challenge", "audit_rebuttal"] {
        assert!(kinds.contains(&k.to_string()), "{k} in {kinds:?}");
    }
    // Done: the tools are the owner's again
    let (_, err, text) = tool(&e, &owner, "task_request_check", json!({"task_id": t.id, "kind": "file_exists", "spec": {"path": "y"}})).await;
    assert!(!err || !text.contains("during its audit"), "{text}");
}

#[tokio::test]
async fn no_risks_raised_is_not_a_pass() {
    let e = setup().await;
    let owner = bot(&e, "Owner").await;
    auditor_on(&e, 5).await;
    let t = task(&e, Some(&owner), 0, "review").await;
    let before = verification(&e, &t.id).await;
    let a = open_audit(&e, &t).await;
    let challenger = e.state.db.get_kwant(&a.challenger_kwant_id).await.unwrap().unwrap();
    let (v, err, text) = tool(&e, &challenger, "audit_challenge", json!({"audit_id": a.id, "risks": []})).await;
    assert!(!err && v["next"].as_str().unwrap().contains("isn't a pass"), "{text}");
    let done = e.state.db.get_audit(&a.id).await.unwrap().unwrap();
    assert_eq!((done.status.as_str(), done.reason.as_deref()), ("done", Some("no risks raised")));
    assert_eq!(verification(&e, &t.id).await, before, "still in review, nothing verified");
}

#[tokio::test]
async fn the_person_closes_risks_and_bots_cannot() {
    let e = setup().await;
    let owner = bot(&e, "Owner").await;
    auditor_on(&e, 5).await;
    let t = task(&e, Some(&owner), 2, "review").await;
    let a = open_audit(&e, &t).await;
    let challenger = e.state.db.get_kwant(&a.challenger_kwant_id).await.unwrap().unwrap();
    let risks = json!({"audit_id": a.id, "risks": [{"risk": "r1", "evidence": "e1"}, {"risk": "r2", "evidence": "e2"}]});
    let (v, err, text) = tool(&e, &challenger, "audit_challenge", risks).await;
    assert!(!err, "{text}");
    let r1 = v["risks"][0]["id"].as_str().unwrap().to_string();
    let space = person(&e, "GET", "/api/risks", None).await;
    assert_eq!(space["risks"].as_array().unwrap().len(), 2, "{space}");

    // A bot's token can't start an audit or close a risk
    let tok = token(&e, &owner).await;
    let (status, _) = call(&e.app, "POST", &format!("/api/tasks/{}/risks/{r1}/resolve", t.id), Some(&tok), Some(json!({"resolution": "dismissed"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&e.app, "POST", &format!("/api/tasks/{}/audit", t.id), Some(&tok), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(&e.app, "POST", &format!("/api/tasks/{}/risks/{r1}/resolve", t.id), None, Some(json!({"resolution": "verified"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "only resolved or dismissed");
    let (status, _) = call(&e.app, "POST", &format!("/api/tasks/other/risks/{r1}/resolve"), None, Some(json!({"resolution": "resolved"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the risk must be the task's");
    let v = person(&e, "POST", &format!("/api/tasks/{}/risks/{r1}/resolve", t.id), Some(json!({"resolution": "resolved", "note": "Fixed in 2.1"}))).await;
    assert_eq!((v["risk"]["status"].as_str(), v["unresolved"].as_bool()), (Some("resolved"), Some(false)), "{v}");
    let (status, _) = call(&e.app, "POST", &format!("/api/tasks/{}/risks/{r1}/resolve", t.id), None, Some(json!({"resolution": "dismissed"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "already closed");

    assert_eq!(person(&e, "GET", &format!("/api/tasks/{}/risks", t.id), None).await["open_risks"], 1);
    assert_eq!(person(&e, "GET", "/api/risks", None).await["risks"].as_array().unwrap().len(), 1);
    assert!(events(&e, &t.id).await.contains(&"risk_resolved".to_string()));
    assert_eq!(e.state.db.get_task(&t.id).await.unwrap().unwrap().state, "review", "closing a risk doesn't move the task");
}

/// A bot whose model calls go to a port nothing listens on, in a space of its own.
async fn dead_api_bot(e: &Env, space: &str, name: &str) -> Kwant {
    let (s, v) = call(
        &e.app,
        "POST",
        &format!("/api/profiles?space={space}"),
        None,
        Some(json!({
            "engine": "custom", "kind": "custom_api", "name": format!("Dead {name}"),
            "protocol": "openai-chat", "base_url": "http://127.0.0.1:9/v1",
            "api_key": "sk-test-0123456789", "model": "gpt-4.1",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = call(
        &e.app,
        "POST",
        &format!("/api/kwants?space={space}"),
        None,
        Some(json!({"name": name, "title": "Tester", "persona_prompt": "Test", "avatar_sprite": "s", "driver_type": "api", "preferred_model": "custom"})),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
    e.state.db.get_kwant(v["id"].as_str().unwrap()).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_failed_audit_turn_leaves_the_task_alone() {
    let e = setup().await;
    let (s, v) = call(&e.app, "POST", "/api/spaces", None, Some(json!({"name": "Audits", "icon": "🧪", "this_machine_enabled": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let space = v["id"].as_str().unwrap().to_string();
    let owner = dead_api_bot(&e, &space, "Owner").await;
    let challenger = dead_api_bot(&e, &space, "Challenger").await;
    verified(&e, &challenger, 50).await;
    auditor_on(&e, 5).await;
    let t = task_in(&e, &space, Some(&owner), 2, "review").await;
    person_check(&e, &t).await;
    let before = verification(&e, &t.id).await;

    let a = auditor::begin(&e.state, &t, "high_impact").await.unwrap();
    assert_eq!(a.challenger_kwant_id, challenger.id);
    let mut done = None;
    for _ in 0..600 {
        let now = e.state.db.get_audit(&a.id).await.unwrap().unwrap();
        if !matches!(now.status.as_str(), "queued" | "challenge" | "rebuttal") {
            done = Some(now);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let done = done.expect("the audit ended");
    assert_eq!(done.status, "failed", "{done:?}");
    assert!(done.reason.as_deref().unwrap_or("").contains("without a challenge"), "{done:?}");
    assert_eq!(verification(&e, &t.id).await, before, "an audit never moves or checks the task");
    assert!(e.state.db.risks_of_audit(&a.id).await.unwrap().is_empty());
    let kinds = events(&e, &t.id).await;
    assert!(kinds.contains(&"audit_done".to_string()), "{kinds:?}");
    assert!(!kinds.iter().any(|k| k == "verified" || k == "failed" || k == "peer_review"), "{kinds:?}");
}

#[tokio::test]
async fn a_queued_audit_of_a_task_sent_back_holds_nothing() {
    let e = setup().await;
    let (owner, stranger) = (bot(&e, "Owner").await, bot(&e, "Stranger").await);
    auditor_on(&e, 5).await;
    let mut t = task(&e, Some(&owner), 2, "review").await;
    let a = auditor::start(&e.state, &t, "high_impact").await.unwrap();
    assert!(auditor::held(&e.state, &owner.id, "task_review", &t).await.is_some());
    assert!(auditor::held(&e.state, &owner.id, "task_list", &t).await.is_none(), "reading is never held");
    assert!(auditor::held(&e.state, &stranger.id, "task_review", &t).await.is_none() || stranger.id == a.challenger_kwant_id);
    // A check sent it back to work before the audit started: the owner can claim it again
    t.state = "working".into();
    assert!(auditor::held(&e.state, &owner.id, "task_claim", &t).await.is_none());
    // Once the challenge is running, both sides stay held until it ends
    e.state.db.move_audit(&a.id, "queued", "challenge", None).await.unwrap();
    assert!(auditor::held(&e.state, &owner.id, "task_claim", &t).await.is_some());
    e.state.db.move_audit(&a.id, "challenge", "failed", Some("test")).await.unwrap();
    assert!(auditor::held(&e.state, &owner.id, "task_claim", &t).await.is_none());
}
