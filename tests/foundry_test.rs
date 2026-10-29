//! The Skill Foundry (Phase D3): which clusters get a draft, `skill_draft` validation, replays as
//! dry runs on a scratch copy, rollback within a lineage, restarts, and that a Foundry skill
//! never grants tools or approvals. Off by default. No test here runs a model turn.
//! Contract: docs/mission-control.md §19.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        foundry::NewDraft,
        missions::{NewTask, Task},
        models::{Kwant, NewKwant},
        performance::NewObservation,
        skills::clean_skill_text,
        spaces::SpaceId,
        Database,
    },
    mcp::{ApprovalOutcome, BuiltinCall, ToolApprovalRequest, ToolApprover, SELF_SERVER_NAME},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::foundry,
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
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
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

async fn foundry_on(e: &Env, drafts: i64) {
    person(e, "POST", "/api/settings", Some(json!({"skill_foundry": "on", "skill_foundry_daily_drafts": drafts}))).await;
}

/// A bot of the default space. Its engine is Codex, so a replay of it is refused before any
/// model runs (replays need Claude's permission hook), and no test here starts a real turn.
async fn bot(e: &Env, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Tester".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "codex".into(),
    };
    e.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn task(e: &Env, owner: &Kwant, title: &str) -> Task {
    let t = NewTask {
        title: title.into(),
        acceptance: "The notes file lists every change".into(),
        state: "ready".into(),
        priority: 1,
        owner_kwant_id: Some(owner.id.clone()),
        budget_tokens: 0,
        max_attempts: 3,
        source_kind: "person".into(),
        created_by: "person".into(),
        ..Default::default()
    };
    e.state.db.create_task(&SpaceId::default_space(), &t).await.unwrap()
}

/// A Performance Market row: `bot` ended task `task_id` of `class` with `result`, `days_ago`.
async fn observe(e: &Env, bot: &Kwant, task_id: &str, class: &str, result: &'static str, days_ago: i64) {
    let o = NewObservation {
        task_id: task_id.into(),
        space_id: bot.space_id.clone(),
        mission_id: None,
        task_class: class.into(),
        kwant_id: bot.id.clone(),
        engine: "cli".into(),
        model: "m".into(),
        started_at: None,
        finished_at: chrono::Utc::now() - chrono::Duration::days(days_ago),
        latency_ms: 0,
        runs: 1,
        fresh_tokens: 0,
        cost_usd: None,
        result,
        approvals: 0,
        check_approvals: 0,
        unblocks: 0,
        retries: 0,
        autofix_attempts: 0,
    };
    e.state.db.store_observation(&o, "live").await.unwrap();
}

/// `n` verified tasks of `class` by `bot`, as real tasks; returns their ids.
async fn verified(e: &Env, bot: &Kwant, class: &str, n: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..n {
        let t = task(e, bot, &format!("{class} {i}")).await;
        observe(e, bot, &t.id, class, "verified", 1).await;
        ids.push(t.id);
    }
    ids
}

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

async fn rpc(e: &Env, bot: &Kwant, method: &str, params: Value) -> Value {
    let t = token(e, bot).await;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(&t), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}

async fn listed(e: &Env, bot: &Kwant) -> Vec<String> {
    let v = rpc(e, bot, "tools/list", json!({})).await;
    v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

async fn tool(e: &Env, bot: &Kwant, name: &str, args: Value) -> (Value, bool, String) {
    let v = rpc(e, bot, "tools/call", json!({"name": name, "arguments": args})).await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    (serde_json::from_str(&text).unwrap_or(Value::Null), v["result"]["isError"].as_bool().unwrap_or(false), text)
}

fn cluster(bot: &Kwant, class: &str) -> kwnt::db::foundry::Cluster {
    kwnt::db::foundry::Cluster { space_id: bot.space_id.clone(), kwant_id: bot.id.clone(), task_class: class.into(), verified: 3, total: 3 }
}

/// A draft run of `bot` for `class` waiting for `skill_draft`, opened without running its turn.
async fn drafting(e: &Env, bot: &Kwant, class: &str) -> kwnt::db::foundry::FoundryRun {
    let run = foundry::start_draft(&e.state, &cluster(bot, class), "test").await.unwrap();
    assert!(e.state.db.move_foundry_run(&run.id, "queued", "drafting", None).await.unwrap());
    run
}

fn draft_args(run_id: &str, examples: &[String]) -> Value {
    json!({
        "run_id": run_id,
        "name": "release-notes",
        "description": "Write release notes from the merged changes",
        "body": "1. List merged changes.\n2. Write NOTES.md.\n3. Check every change is listed.",
        "inputs": [{"name": "range", "description": "The commits to cover", "example": "v1.2..main"}],
        "checks": [{"kind": "file_exists", "spec": {"path": "NOTES.md"}}],
        "examples": examples,
        "tools": ["Read", "Write", "task_list"],
    })
}

/// A Foundry draft stored straight in the database (no turn), as version `n` of `bot`'s class.
async fn stored_draft(e: &Env, bot: &Kwant, class: &str, body: &str, examples: &[String]) -> (kwnt::db::skills::Skill, kwnt::db::foundry::SkillMeta) {
    let run = e.state.db.create_foundry_run("draft", &bot.space_id, &bot.id, class, None).await.unwrap();
    let draft = NewDraft {
        kwant_id: bot.id.clone(),
        task_class: class.into(),
        text: clean_skill_text("release-notes", "Write release notes", body).unwrap(),
        inputs: json!([]),
        checks: json!([{"kind": "file_exists", "spec": {"path": "NOTES.md"}}, {"kind": "http", "spec": {"url": "https://example.com/health"}}]),
        examples: examples.to_vec(),
        tools: vec!["Read".into()],
    };
    e.state.db.insert_foundry_draft(&bot.space_id, &draft, &run.id).await.unwrap()
}

#[tokio::test]
async fn the_foundry_is_off_by_default() {
    let e = setup().await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!(s["skill_foundry"], "off", "{s}");
    assert_eq!(
        (&s["skill_foundry_model"], &s["skill_foundry_daily_drafts"], &s["skill_foundry_replays"], &s["skill_foundry_replay_tokens"]),
        (&json!("balanced"), &json!(2), &json!(3), &json!(60_000)),
        "{s}"
    );
    let owner = bot(&e, "Owner").await;
    verified(&e, &owner, "release", 3).await;

    // Clusters are shown, but no draft is started, by the tick or the person
    let o = person(&e, "GET", "/api/skills/foundry", None).await;
    assert_eq!((o["enabled"].as_bool(), o["clusters"].as_array().unwrap().len()), (Some(false), 1), "{o}");
    assert!(foundry::tick(&e.state).await.is_empty());
    let uri = format!("/api/skills/foundry/{}/draft", o["clusters"][0]["id"].as_str().unwrap());
    let (status, v) = call(&e.app, "POST", &uri, None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(foundry::start_draft(&e.state, &cluster(&owner, "release"), "test").await.unwrap_err().contains("off"));
    assert!(e.state.db.foundry_runs_of_space(&owner.space_id, 10).await.unwrap().is_empty());
    // The bot is never offered skill_draft, and calling it is refused
    assert!(!listed(&e, &owner).await.contains(&foundry::TOOL.to_string()));
    let (_, err, text) = tool(&e, &owner, foundry::TOOL, json!({"run_id": "x"})).await;
    assert!(err && text.contains("off"), "{text}");
}

#[tokio::test]
async fn settings_are_validated() {
    let e = setup().await;
    for bad in [
        json!({"skill_foundry": "auto"}),
        json!({"skill_foundry_model": "opus"}),
        json!({"skill_foundry_daily_drafts": 0}),
        json!({"skill_foundry_replays": 6}),
        json!({"skill_foundry_replay_tokens": 100}),
    ] {
        let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} {v}");
    }
    person(&e, "POST", "/api/settings", Some(json!({"skill_foundry": "on", "skill_foundry_model": "fast", "skill_foundry_replays": 0}))).await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((&s["skill_foundry"], &s["skill_foundry_model"], &s["skill_foundry_replays"]), (&json!("on"), &json!("fast"), &json!(0)), "{s}");
}

#[tokio::test]
async fn the_miner_needs_three_verified_tasks_of_one_class_at_seventy_percent_in_sixty_days() {
    let e = setup().await;
    let (a, b, c, d, f) = (bot(&e, "A").await, bot(&e, "B").await, bot(&e, "C").await, bot(&e, "D").await, bot(&e, "F").await);
    // A: 3 verified, 1 failed (75%): a cluster
    verified(&e, &a, "release", 3).await;
    observe(&e, &a, "a-failed", "release", "failed", 2).await;
    // B: only 2 verified
    verified(&e, &b, "release", 2).await;
    // C: 3 verified of 5 (60%)
    verified(&e, &c, "release", 3).await;
    observe(&e, &c, "c-f1", "release", "failed", 1).await;
    observe(&e, &c, "c-f2", "release", "failed", 1).await;
    // D: `general` is never a cluster
    verified(&e, &d, "general", 5).await;
    // F: 3 verified, but one of them 61 days ago
    verified(&e, &f, "release", 2).await;
    observe(&e, &f, "f-old", "release", "verified", 61).await;

    let clusters = e.state.db.foundry_clusters(None, chrono::Utc::now()).await.unwrap();
    let ids: Vec<(String, String)> = clusters.iter().map(|c| (c.kwant_id.clone(), c.task_class.clone())).collect();
    assert_eq!(ids, vec![(a.id.clone(), "release".to_string())], "{clusters:?}");
    assert_eq!((clusters[0].verified, clusters[0].total), (3, 4));
    // Another space sees none of them
    assert!(e.state.db.foundry_clusters(Some("other-space"), chrono::Utc::now()).await.unwrap().is_empty());
}

#[tokio::test]
async fn drafts_are_capped_and_a_class_cools_off() {
    let e = setup().await;
    foundry_on(&e, 1).await;
    let (a, b) = (bot(&e, "A").await, bot(&e, "B").await);
    let now = chrono::Utc::now();
    assert_eq!(e.state.db.foundry_block(&a.id, "release", now).await.unwrap(), None);

    let run = foundry::start_draft(&e.state, &cluster(&a, "release"), "test").await.unwrap();
    assert_eq!((run.kind.as_str(), run.status.as_str()), ("draft", "queued"));
    // A second live draft for the class is refused; the daily cap stops other classes too
    assert!(foundry::start_draft(&e.state, &cluster(&a, "release"), "test").await.is_err());
    assert!(foundry::start_draft(&e.state, &cluster(&b, "release"), "test").await.unwrap_err().contains("daily"));
    // The tick offers nothing more today, and the class cools off for a week
    assert!(foundry::tick(&e.state).await.is_empty());
    assert_eq!(e.state.db.foundry_block(&a.id, "release", now).await.unwrap(), Some("cooling_off"));
    assert_eq!(e.state.db.foundry_block(&a.id, "release", now + chrono::Duration::days(8)).await.unwrap(), None);
}

#[tokio::test]
async fn a_rejected_draft_is_not_offered_again_for_thirty_days() {
    let e = setup().await;
    let a = bot(&e, "A").await;
    let ex = verified(&e, &a, "release", 3).await;
    let (s, _) = stored_draft(&e, &a, "release", "Steps.", &ex[..2]).await;
    let now = chrono::Utc::now();
    assert_eq!(e.state.db.foundry_block(&a.id, "release", now).await.unwrap(), Some("has_draft"));
    person(&e, "POST", &format!("/api/skills/{}/reject", s.id), Some(json!({}))).await;
    // The draft run itself was today: past its week, the rejection still blocks
    let later = now + chrono::Duration::days(10);
    assert_eq!(e.state.db.foundry_block(&a.id, "release", later).await.unwrap(), Some("rejected"));
    assert_eq!(e.state.db.foundry_block(&a.id, "release", now + chrono::Duration::days(31)).await.unwrap(), None);
}

#[tokio::test]
async fn skill_draft_stores_a_pending_draft_and_refuses_bad_parts() {
    let e = setup().await;
    foundry_on(&e, 5).await;
    let (owner, other) = (bot(&e, "Owner").await, bot(&e, "Other").await);
    let ex = verified(&e, &owner, "release", 3).await;
    let foreign = verified(&e, &owner, "invoices", 1).await;
    let others = verified(&e, &other, "release", 1).await;
    assert!(!listed(&e, &owner).await.contains(&foundry::TOOL.to_string()));
    let run = drafting(&e, &owner, "release").await;
    // Offered only to the bot with the open draft run
    assert!(listed(&e, &owner).await.contains(&foundry::TOOL.to_string()));
    assert!(!listed(&e, &other).await.contains(&foundry::TOOL.to_string()));

    let refused = |args: Value, why: &'static str| {
        let (e, owner) = (&e, &owner);
        async move {
            let (_, err, text) = tool(e, owner, foundry::TOOL, args).await;
            assert!(err && text.contains(why), "{why}: {text}");
        }
    };
    let good = draft_args(&run.id, &ex[..2]);
    // Examples must be the cluster's own verified tasks, 2–5, no repeats
    let mut a = good.clone();
    a["examples"] = json!([ex[0], foreign[0]]);
    refused(a, "isn't one of your verified").await;
    let mut a = good.clone();
    a["examples"] = json!([ex[0], others[0]]);
    refused(a, "isn't one of your verified").await;
    let mut a = good.clone();
    a["examples"] = json!([ex[0]]);
    refused(a, "2–5").await;
    let mut a = good.clone();
    a["examples"] = json!([ex[0], ex[0]]);
    refused(a, "twice").await;
    // Tools must be ones the bot holds now
    let mut a = good.clone();
    a["tools"] = json!(["Read", "mcp__github__create_issue"]);
    refused(a, "don't have a tool called mcp__github__create_issue").await;
    let mut a = good.clone();
    a["tools"] = json!(["TotallyNew"]);
    refused(a, "don't have a tool called TotallyNew").await;
    // Checks pass the task-check validator
    let mut a = good.clone();
    a["checks"] = json!([{"kind": "person", "spec": {}}]);
    refused(a, "kind must be one of").await;
    let mut a = good.clone();
    a["checks"] = json!([{"kind": "http", "spec": {"url": "file:///etc/passwd"}}]);
    refused(a, "check 1").await;
    // Someone else's run
    let mut a = good.clone();
    a["run_id"] = json!("nope");
    refused(a, "No such").await;
    let (_, err, _) = tool(&e, &other, foundry::TOOL, good.clone()).await;
    assert!(err);

    let (v, err, text) = tool(&e, &owner, foundry::TOOL, good.clone()).await;
    assert!(!err, "{text}");
    let skill_id = v["skill_id"].as_str().unwrap().to_string();
    let s = e.state.db.get_skill(&skill_id).await.unwrap().unwrap();
    assert_eq!((s.status.as_str(), s.name.as_str(), s.kwant_id.as_deref()), ("pending", "release-notes", Some(owner.id.as_str())));
    let m = e.state.db.skill_meta(&skill_id).await.unwrap().unwrap();
    assert_eq!((m.version, m.lineage_id.as_str(), m.task_class.as_str()), (1, skill_id.as_str(), "release"));
    assert_eq!(serde_json::from_str::<Value>(&m.tools).unwrap(), json!(["Read", "Write", "task_list"]));
    assert_eq!(m.examples(), ex[..2].to_vec());
    // The run holds the draft; a second call is refused
    assert_eq!(e.state.db.get_foundry_run(&run.id).await.unwrap().unwrap().skill_id.as_deref(), Some(skill_id.as_str()));
    refused(good, "isn't waiting").await;
    // Each example's history links to the draft; the draft has its own event
    let kinds: Vec<String> = e.state.db.task_events(&ex[0], 50).await.unwrap().into_iter().map(|ev| ev.kind).collect();
    assert!(kinds.contains(&"skill_drafted_from".to_string()), "{kinds:?}");
    let events = e.state.db.skill_events(std::slice::from_ref(&skill_id)).await.unwrap();
    assert_eq!(events.iter().map(|ev| ev.kind.as_str()).collect::<Vec<_>>(), ["skill_drafted"]);
    // And the card shows it
    let o = person(&e, "GET", "/api/skills/foundry", None).await;
    assert_eq!(o["skills"][0]["foundry"]["tools"], json!(["Read", "Write", "task_list"]), "{o}");
    assert_eq!(o["clusters"][0]["block"], "has_draft", "{o}");
}

#[tokio::test]
async fn a_foundry_skill_grants_no_tools_or_approvals() {
    let e = setup().await;
    foundry_on(&e, 5).await;
    let owner = bot(&e, "Owner").await;
    let ex = verified(&e, &owner, "release", 3).await;
    let before_tools = listed(&e, &owner).await;
    let before_bot = serde_json::to_value(e.state.db.get_kwant(&owner.id).await.unwrap().unwrap()).unwrap();
    let before_servers = e.state.db.list_active_mcp_servers_for_kwant(&owner.id).await.unwrap().len();

    let run = drafting(&e, &owner, "release").await;
    let mut args = draft_args(&run.id, &ex[..2]);
    args["tools"] = json!(["Read", "Write", "Bash", "WebFetch"]);
    let (v, err, text) = tool(&e, &owner, foundry::TOOL, args).await;
    assert!(!err, "{text}");
    let id = v["skill_id"].as_str().unwrap().to_string();
    let _ = e.state.db.move_foundry_run(&run.id, "drafting", "done", None).await;
    person(&e, "POST", &format!("/api/skills/{id}/approve"), Some(json!({}))).await;
    assert_eq!(e.state.db.get_skill(&id).await.unwrap().unwrap().status, "enabled");

    assert_eq!(listed(&e, &owner).await, before_tools);
    let after_bot = serde_json::to_value(e.state.db.get_kwant(&owner.id).await.unwrap().unwrap()).unwrap();
    assert_eq!(after_bot, before_bot);
    assert_eq!(e.state.db.list_active_mcp_servers_for_kwant(&owner.id).await.unwrap().len(), before_servers);
}

#[tokio::test]
async fn replay_mode_refuses_every_outward_call_and_records_it() {
    let log: mcp_runtime::RehearseLog = Default::default();
    let approver = mcp_runtime::RehearseApprover { log: log.clone() };
    let req = |server: &str, tool: &str, args: Value, builtin: Option<BuiltinCall>| ToolApprovalRequest {
        kwant_id: "k".into(),
        kwant_name: "K".into(),
        turn_id: "t".into(),
        tool_name: if builtin.is_some() { tool.to_string() } else { format!("mcp__{server}__{tool}") },
        server: server.into(),
        tool: tool.into(),
        arguments: args,
        builtin,
        smart: None,
        payment: None,
    };
    let inside = || Some(BuiltinCall { cwd: Some("/scratch".into()), outside_folder: false });
    let outside = || Some(BuiltinCall { cwd: Some("/scratch".into()), outside_folder: true });

    // Reading kwant's own records is allowed
    assert!(matches!(approver.decide(req(SELF_SERVER_NAME, "task_list", json!({}), None)).await, ApprovalOutcome::Allowed));
    // Everything else that reaches the approver is refused as rehearsed, never asked
    for r in [
        req("", "Bash", json!({"command": "git push"}), inside()),
        req("", "Write", json!({"file_path": "/home/x/.ssh/config", "content": "x"}), outside()),
        req("gmail", "send_email", json!({"to": "a@b.c"}), None),
        req(SELF_SERVER_NAME, "task_update", json!({"task_id": "t1", "status": "done"}), None),
        req(SELF_SERVER_NAME, "browser_act", json!({"action": "click"}), None),
    ] {
        let name = r.tool_name.clone();
        let outcome = approver.decide(r).await;
        assert!(matches!(outcome, ApprovalOutcome::Rehearsed), "{name}: {outcome:?}");
        assert!(!outcome.allowed());
    }
    let lines = log.lock().unwrap().clone();
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[0]["tool"], "Bash");
    assert_eq!(lines[1]["outside_folder"], true);
    assert_eq!(lines[2]["tool"], "mcp__gmail__send_email");
    // Subagents are gated even where the hook would allow them
    assert!(approver.tier_gate(&req("", "Task", json!({}), inside())).await.is_some());
    assert!(approver.tier_gate(&req("", "Read", json!({}), inside())).await.is_none());
    // The refusal tells the bot it was a dry run
    assert!(ApprovalOutcome::Rehearsed.refusal_text().contains("dry run"));
}

#[tokio::test]
async fn scratch_checks_run_file_checks_and_mark_network_ones_not_replayable() {
    let e = setup().await;
    let lead = bot(&e, "Lead").await;
    let m = person(
        &e,
        "POST",
        "/api/missions",
        Some(json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": lead.id, "budget_tokens": 900_000,
                    "channel_id": e.general, "tasks": [{"title": "Notes", "owner": lead.id}]})),
    )
    .await;
    let mid = m["id"].as_str().or(m["mission"]["id"].as_str()).unwrap().to_string();
    let detail = person(&e, "GET", &format!("/api/missions/{mid}"), None).await;
    let tid = detail["tasks"][0]["id"].as_str().unwrap().to_string();
    let folder = std::path::PathBuf::from(e.state.db.get_mission(&mid).await.unwrap().unwrap().workspace_path);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("NOTES.md"), "v1.2: export fixed").unwrap();
    person(&e, "POST", &format!("/api/tasks/{tid}/checkpoints"), Some(json!({}))).await;
    // The live folder moves on; the replay sees the snapshot
    std::fs::write(folder.join("NOTES.md"), "changed later").unwrap();

    let scratch = foundry::scratch_dir(&e.state, "run1", 0);
    let n = kwnt::server::missions::checkpoints::materialize_first(&e.state, &tid, &scratch).await.unwrap();
    assert_eq!(n, 1);
    assert_eq!(std::fs::read_to_string(scratch.join("NOTES.md")).unwrap(), "v1.2: export fixed");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&scratch).unwrap().permissions().mode() & 0o777, 0o700);

    let checks = vec![
        json!({"kind": "file_exists", "spec": {"path": "NOTES.md"}}),
        json!({"kind": "file_contains", "spec": {"path": "NOTES.md", "text": "export fixed"}}),
        json!({"kind": "http", "spec": {"url": "https://example.com/health"}}),
        json!({"kind": "command", "spec": {"command": "curl https://example.com"}}),
    ];
    let out = foundry::scratch_checks(&checks, &scratch).await;
    let statuses: Vec<&str> = out.iter().map(|c| c["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["passed", "passed", "not_replayable", "not_replayable"], "{out:?}");
    assert_eq!(foundry::example_verdict(&out), "passed");
    let missing = foundry::scratch_checks(&[json!({"kind": "file_exists", "spec": {"path": "OTHER.md"}})], &scratch).await;
    assert_eq!(foundry::example_verdict(&missing), "failed");
    // A task with no checkpoint replays on an empty folder
    assert_eq!(kwnt::server::missions::checkpoints::materialize_first(&e.state, "no-such-task", &scratch.join("x")).await.unwrap(), 0);
}

#[tokio::test]
async fn a_replay_off_claude_is_reported_as_an_error_and_leaves_no_scratch_copy() {
    let e = setup().await;
    foundry_on(&e, 5).await;
    let owner = bot(&e, "Owner").await;
    let ex = verified(&e, &owner, "release", 3).await;
    let (s, _) = stored_draft(&e, &owner, "release", "Steps.", &ex[..2]).await;

    let run = foundry::begin_replay(&e.state, &s.id, "test").await.unwrap();
    // A second replay of the same draft waits for the first
    assert!(foundry::begin_replay(&e.state, &s.id, "test").await.is_err());
    let mut done = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let r = e.state.db.get_foundry_run(&run.id).await.unwrap().unwrap();
        if !r.live() {
            done = Some(r);
            break;
        }
    }
    let r = done.expect("the replay never ended");
    assert_eq!((r.status.as_str(), r.reason.as_deref()), ("failed", Some("no example could be replayed")), "{r:?}");
    let report: Value = serde_json::from_str(&e.state.db.skill_meta(&s.id).await.unwrap().unwrap().replay.unwrap()).unwrap();
    assert_eq!(report["examples"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(report["errors"], 2, "{report}");
    assert!(report["examples"][0]["error"].as_str().unwrap().contains("only on Claude"), "{report}");
    // The draft is untouched, and its scratch copies are gone
    assert_eq!(e.state.db.get_skill(&s.id).await.unwrap().unwrap().status, "pending");
    assert!(!e.state.config.data_dir.join("foundry").join(&run.id).exists());
    let kinds: Vec<String> = e.state.db.skill_events(std::slice::from_ref(&s.id)).await.unwrap().into_iter().map(|ev| ev.kind).collect();
    assert_eq!(kinds, ["skill_drafted", "skill_replayed"]);
    // Only a draft waiting for review is replayed
    person(&e, "POST", &format!("/api/skills/{}/approve", s.id), Some(json!({}))).await;
    let (status, v) = call(&e.app, "POST", &format!("/api/skills/{}/replay", s.id), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn rollback_moves_one_lineage_back_in_one_step() {
    let e = setup().await;
    let (a, b) = (bot(&e, "A").await, bot(&e, "B").await);
    let ex = verified(&e, &a, "release", 3).await;
    let (v1, m1) = stored_draft(&e, &a, "release", "Version one.", &ex[..2]).await;
    person(&e, "POST", &format!("/api/skills/{}/approve", v1.id), Some(json!({}))).await;
    let (v2, m2) = stored_draft(&e, &a, "release", "Version two.", &ex[..2]).await;
    assert_eq!((m2.version, m2.lineage_id.as_str(), v2.replaces_id.as_deref()), (2, m1.lineage_id.as_str(), Some(v1.id.as_str())));
    person(&e, "POST", &format!("/api/skills/{}/approve", v2.id), Some(json!({}))).await;
    assert_eq!(e.state.db.get_skill(&v1.id).await.unwrap().unwrap().status, "superseded");
    // Another bot's live skill of the same class is another lineage
    let exb = verified(&e, &b, "release", 3).await;
    let (other, _) = stored_draft(&e, &b, "release", "B's way.", &exb[..2]).await;
    person(&e, "POST", &format!("/api/skills/{}/approve", other.id), Some(json!({}))).await;

    // A bot can't roll back
    let t = token(&e, &a).await;
    let (status, _) = call(&e.app, "POST", &format!("/api/skills/{}/rollback", v1.id), Some(&t), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The live version can't be "rolled back" to
    let (status, v) = call(&e.app, "POST", &format!("/api/skills/{}/rollback", v2.id), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");

    let r = person(&e, "POST", &format!("/api/skills/{}/rollback", v1.id), None).await;
    assert_eq!(r["status"], "enabled", "{r}");
    let status = |id: String| {
        let e = &e;
        async move { e.state.db.get_skill(&id).await.unwrap().unwrap().status }
    };
    assert_eq!(status(v2.id.clone()).await, "superseded");
    assert_eq!(status(other.id.clone()).await, "enabled");
    let d = person(&e, "GET", &format!("/api/skills/{}/foundry", v1.id), None).await;
    assert_eq!(d["versions"].as_array().unwrap().len(), 2, "{d}");
    assert!(d["events"].as_array().unwrap().iter().any(|ev| ev["kind"] == "skill_rolled_back"), "{d}");
    // And forward again
    person(&e, "POST", &format!("/api/skills/{}/rollback", v2.id), None).await;
    assert_eq!((status(v1.id.clone()).await, status(v2.id.clone()).await), ("superseded".into(), "enabled".into()));
    // A skill without Foundry history isn't the Foundry's to roll back
    let plain = person(&e, "POST", "/api/skills", Some(json!({"name": "plain", "description": "d", "body": "b"}))).await;
    let (status, _) = call(&e.app, "POST", &format!("/api/skills/{}/rollback", plain["id"].as_str().unwrap()), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reads_of_the_review_queue_are_person_only() {
    let e = setup().await;
    let a = bot(&e, "A").await;
    let t = token(&e, &a).await;
    for (method, uri) in [("GET", "/api/skills/foundry"), ("POST", "/api/skills/foundry/x~y/draft"), ("GET", "/api/skills/s1/foundry"), ("POST", "/api/skills/s1/replay")] {
        let (status, _) = call(&e.app, method, uri, Some(&t), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn queued_runs_fail_at_startup_when_the_foundry_was_turned_off() {
    let e = setup().await;
    let a = bot(&e, "A").await;
    let queued = e.state.db.create_foundry_run("draft", &a.space_id, &a.id, "release", None).await.unwrap();
    foundry::resume_after_restart(&e.state).await;
    let r = e.state.db.get_foundry_run(&queued.id).await.unwrap().unwrap();
    assert_eq!((r.status.as_str(), r.reason.as_deref()), ("failed", Some("the Skill Foundry was turned off")));
    // A run whose driver stops ends failed
    let live = e.state.db.create_foundry_run("draft", &a.space_id, &a.id, "invoices", None).await.unwrap();
    assert!(foundry::end_if_live(&e.state, &live.id, "stopped").await);
    assert!(!foundry::end_if_live(&e.state, &live.id, "stopped").await);
}
