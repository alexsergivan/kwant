//! Truth Ledger C2, `on` (Mission Control Phase C2): the bot's memory block is read from the
//! ledger and labelled, the deterministic contradiction scan opens disputes ("4 GB" vs "12 GB";
//! not paraphrases), a dispute is settled by the person or by a passing kwant-run check, tasks
//! that rely on a disputed fact land in Needs you, and `off`/`shadow` stay byte-identical to main
//! (fixtures/truth_ledger_off_prompt_895a96c.txt).
//! Contract: docs/mission-control.md §21.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        missions::{NewCheck, NewTask, Task},
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{prompt_block, SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::{self, truth_ledger},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    general: String,
    pool: SqlitePool,
    _tmp: TempDir,
}

async fn open(data: &Path) -> (Router, AppState, SqlitePool) {
    let db = Database::init(data).await.unwrap();
    let config = Config { data_dir: data.to_path_buf(), laya_url: "http://127.0.0.1:9".into(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let pool = SqlitePoolOptions::new().max_connections(1).connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    (create_router(state.clone()), state, pool)
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let (app, state, pool) = open(&data).await;
    let general = state.db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app, state, general, pool, _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, key: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = key {
        req = req.header("idempotency-key", key);
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = call(&e.app, method, uri, None, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn ledger(e: &Env, mode: &str) {
    person(e, "POST", "/api/settings", Some(json!({"truth_ledger": mode}))).await;
}

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

/// A turn for `bot`: its id and the bot's token.
async fn turn(e: &Env, bot: &Kwant) -> (String, String) {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), origin: Some(origin), turn_id: Some(turn_id.clone()), ..Default::default() };
    let token = mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.clone()), None, Some(scope)).await.unwrap().1;
    (turn_id, token)
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> String {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), None, Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_ne!(v["result"]["isError"], json!(true), "{name}: {v}");
    v["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

/// `Saved as [id].` → id
fn saved_id(reply: &str) -> String {
    reply.split('[').nth(1).and_then(|s| s.split(']').next()).unwrap().to_string()
}

async fn live_fact(e: &Env, memory_id: &str) -> (String, String, String) {
    sqlx::query_as("SELECT id, trust, status FROM knowledge_facts WHERE memory_id = ? AND status <> 'superseded'")
        .bind(memory_id)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(sql).fetch_one(pool).await.unwrap().0
}

async fn ledger_on(e: &Env) {
    ledger(e, "on").await;
}

/// The bot's prompt block as a turn would see it.
async fn prompt(e: &Env, k: &Kwant) -> String {
    let k = e.state.db.get_kwant(&k.id).await.unwrap().unwrap();
    prompt_block(&e.state, &k, None, Some(&e.general), true).await
}

async fn open_disputes(e: &Env) -> Vec<Value> {
    person(e, "GET", "/api/facts/disputes", None).await["disputes"].as_array().cloned().unwrap_or_default()
}

async fn status_of(e: &Env, fact_id: &str) -> String {
    sqlx::query_as::<_, (String,)>("SELECT status FROM knowledge_facts WHERE id = ?").bind(fact_id).fetch_one(&e.pool).await.unwrap().0
}

async fn task(e: &Env, owner: &Kwant, state: &str) -> Task {
    let t = NewTask {
        title: "Size the server".into(),
        acceptance: "It fits".into(),
        state: state.into(),
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

/// A task verified by a passing kwant-run check.
async fn verified_task(e: &Env, owner: &Kwant) -> Task {
    let t = task(e, owner, "review").await;
    check(e, &t, "passed").await;
    sqlx::query("UPDATE tasks SET state = 'verified' WHERE id = ?").bind(&t.id).execute(&e.pool).await.unwrap();
    e.state.db.get_task(&t.id).await.unwrap().unwrap()
}

/// A kwant-run check on `t` that finished with `status`.
async fn check(e: &Env, t: &Task, status: &str) -> String {
    let c = e
        .state
        .db
        .add_check(&NewCheck {
            task_id: t.id.clone(),
            kind: "file_exists".into(),
            spec: json!({"path": "ram.txt"}),
            required: true,
            approved: true,
            requested_by: "person".into(),
            approved_by: Some("person".into()),
        })
        .await
        .unwrap();
    e.state.db.start_check(&c.id).await.unwrap();
    e.state.db.finish_check(&c.id, status, None, "kwant", None).await.unwrap();
    c.id
}

/// `text` with every UUID replaced by `{id}`.
fn without_ids(text: &str) -> String {
    let shape = |w: &[u8]| {
        w.len() == 36 && w.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_digit() || (b'a'..=b'f').contains(c) })
    };
    let bytes = text.as_bytes();
    let (mut out, mut i) = (String::new(), 0);
    while i < bytes.len() {
        if bytes.len() - i >= 36 && shape(&bytes[i..i + 36]) {
            out.push_str("{id}");
            i += 36;
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

#[test]
fn the_scan_catches_numbers_negation_and_subjects_but_not_paraphrases() {
    use truth_ledger::conflict_text as c;
    assert_eq!(c("The server has 4 GB of RAM", None, "The server has 12 GB of RAM", None), Some("numbers_differ"));
    assert_eq!(c("The server has 4GB of RAM", None, "The server has 12 gb of RAM", None), Some("numbers_differ"));
    assert_eq!(c("Release builds are signed on CI", None, "Release builds aren't signed on CI", None), Some("negation"));
    assert_eq!(c("Standup is at 10", Some("Standup time"), "Daily meeting moved to Tuesdays", Some("standup time")), Some("same_subject"));
    // A shared subject doesn't hide a more specific cause: what a real Claude turn sends to
    // fact_propose ("server RAM" on both) is numbers_differ, as it is without a subject
    assert_eq!(c("The server has 4 GB of RAM", Some("server RAM"), "The server has 12 GB of RAM", Some("Server RAM")), Some("numbers_differ"));
    assert_eq!(c("Server RAM is 4 GB", Some("server RAM"), "The box was upgraded to 12 GB", Some("server RAM")), Some("numbers_differ"));
    assert_eq!(c("Backups are encrypted", Some("backups"), "Backups aren't encrypted", Some("backups")), Some("negation"));
    // The same fact twice, the same numbers, or different subjects: no dispute
    assert_eq!(c("The server has 4 GB of RAM", None, "the server has 4 GB of RAM.", None), None);
    assert_eq!(c("The server has 4 GB of RAM", None, "The server has 4 GB of RAM installed", None), None);
    assert_eq!(c("The server has 4 GB of RAM", None, "The laptop battery lasts 12 hours", None), None);
    // The documented limit: a paraphrase that contradicts is not caught
    assert_eq!(c("Deploys happen on Fridays", None, "We never ship on Friday", None), None);
}

#[tokio::test]
async fn off_and_shadow_prompts_are_byte_identical_and_switching_off_restores_them() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let (turn_id, token) = turn(&e, &a).await;
    e.state.db.add_memory(&a.id, "Person wrote this", "user").await.unwrap();
    tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM", "pinned": true})).await;
    tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await;
    truth_ledger::taint(&turn_id);
    tool(&e, &token, "memory_add", json!({"text": "Read on a web page"})).await;

    let off = prompt(&e, &a).await;
    // Off is main's prompt, not just this build's: the fixture is this exact setup's prompt on
    // main 895a96c (before C2), with ids as {id}
    let golden = include_str!("fixtures/truth_ledger_off_prompt_895a96c.txt");
    assert_eq!(without_ids(&off), golden, "off differs from main's prompt");
    ledger(&e, "shadow").await;
    assert_eq!(prompt(&e, &a).await, off, "shadow is today's prompt");
    // Off and shadow: no dispute is opened, no fact tool offered
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_disputes").await, 0);
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), None, Some(rpc.clone())).await;
    assert!(!v.to_string().contains("fact_propose"), "{v}");

    ledger_on(&e).await;
    let on = prompt(&e, &a).await;
    assert_ne!(on, off);
    assert!(on.contains("(unconfirmed: from outside text, not yet accepted by the person) Read on a web page"), "{on}");
    assert!(on.contains("(disputed with ["), "{on}");
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), None, Some(rpc)).await;
    assert!(v.to_string().contains("fact_propose"), "{v}");

    ledger(&e, "off").await;
    assert_eq!(prompt(&e, &a).await, off, "off is today's prompt again");
    ledger(&e, "shadow").await;
    assert_eq!(prompt(&e, &a).await, off);
}

#[tokio::test]
async fn differing_numbers_open_one_dispute_and_a_paraphrase_none() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    tool(&e, &token, "memory_add", json!({"text": "Deploys happen on Fridays"})).await;
    let reply = tool(&e, &token, "memory_add", json!({"text": "We never ship on Friday"})).await;
    assert!(!reply.contains("disagrees"), "{reply}");
    assert!(open_disputes(&e).await.is_empty());

    let m4 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM"})).await);
    let reply = tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await;
    assert!(reply.contains("disagrees with another fact"), "{reply}");
    let m12 = saved_id(&reply);
    let disputes = open_disputes(&e).await;
    assert_eq!(disputes.len(), 1, "{disputes:?}");
    assert_eq!(disputes[0]["cause"], "numbers_differ");
    assert_eq!(disputes[0]["facts"].as_array().unwrap().len(), 2);
    assert_eq!(live_fact(&e, &m4).await.2, "disputed");
    assert_eq!(live_fact(&e, &m12).await.2, "disputed");
    // The person is told, without the facts' text
    let n: Vec<(String, String)> = sqlx::query_as("SELECT title, body FROM notifications WHERE kind = 'fact_disputed'")
        .fetch_all(&e.pool)
        .await
        .unwrap_or_default();
    for (title, body) in &n {
        assert!(!title.contains("GB") && !body.contains("GB"), "{title} {body}");
    }

    // Scanning again opens nothing new
    tool(&e, &token, "memory_add", json!({"text": "Likes green tea"})).await;
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_disputes").await, 1);
}

#[tokio::test]
async fn fact_propose_with_a_subject_labels_differing_numbers_numbers_differ() {
    // QA's real run: Claude gave both facts the same subject and the dispute read same_subject
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    tool(&e, &token, "fact_propose", json!({"statement": "The server has 4 GB of RAM", "subject": "server RAM", "scope": "bot"})).await;
    let reply = tool(&e, &token, "fact_propose", json!({"statement": "The server has 12 GB of RAM", "subject": "server RAM", "scope": "bot"})).await;
    assert!(reply.contains("disagrees with another fact"), "{reply}");
    let disputes = open_disputes(&e).await;
    assert_eq!(disputes.len(), 1, "{disputes:?}");
    assert_eq!(disputes[0]["cause"], "numbers_differ");
}

#[tokio::test]
async fn a_dismissed_pair_is_never_raised_again_and_both_go_back() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    let m1 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The staging box has 2 CPUs"})).await);
    let m2 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The staging box has 8 CPUs"})).await);
    let d = open_disputes(&e).await[0]["id"].as_str().unwrap().to_string();
    let v = person(&e, "POST", &format!("/api/facts/disputes/{d}/dismiss"), None).await;
    assert_eq!(v["status"], "dismissed");
    assert_eq!(live_fact(&e, &m1).await.2, "accepted");
    assert_eq!(live_fact(&e, &m2).await.2, "accepted");
    tool(&e, &token, "memory_add", json!({"text": "Likes green tea"})).await;
    prompt(&e, &a).await;
    assert!(open_disputes(&e).await.is_empty());
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_disputes").await, 1);
    // A second dismiss is a 409 with the status
    let (status, v) = call(&e.app, "POST", &format!("/api/facts/disputes/{d}/dismiss"), None, None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(v["status"], "dismissed");
}

#[tokio::test]
async fn the_person_resolves_once_and_the_loser_drops_out_of_the_prompt() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    let m4 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM"})).await);
    let m12 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await);
    let (f4, f12) = (live_fact(&e, &m4).await.0, live_fact(&e, &m12).await.0);
    let d = open_disputes(&e).await[0]["id"].as_str().unwrap().to_string();

    let (status, _) = call(&e.app, "POST", &format!("/api/facts/disputes/{d}/resolve"), None, None, Some(json!({"winner": "f_nope"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let app = e.app.clone();
        let (uri, w) = (format!("/api/facts/disputes/{d}/resolve"), f12.clone());
        handles.push(tokio::spawn(async move { call(&app, "POST", &uri, None, None, Some(json!({"winner": w}))).await.0 }));
    }
    let mut codes = Vec::new();
    for h in handles {
        codes.push(h.await.unwrap());
    }
    assert_eq!(codes.iter().filter(|c| **c == StatusCode::OK).count(), 1, "{codes:?}");
    assert!(codes.iter().all(|c| *c == StatusCode::OK || *c == StatusCode::CONFLICT), "{codes:?}");

    assert_eq!(status_of(&e, &f4).await, "superseded");
    assert_eq!(status_of(&e, &f12).await, "accepted");
    let p = prompt(&e, &a).await;
    assert!(p.contains("12 GB") && !p.contains("4 GB"), "{p}");
    assert!(!p.contains("disputed with"), "{p}");
    // memory_search hides the superseded note too
    let hits = tool(&e, &token, "memory_search", json!({"query": "server RAM"})).await;
    assert!(hits.contains("12 GB") && !hits.contains("4 GB"), "{hits}");
}

#[tokio::test]
async fn evidence_settles_a_dispute_only_with_a_passing_run_check_and_never_for_outside() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (turn_id, token) = turn(&e, &a).await;
    let m4 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM"})).await);
    let m12 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await);
    let (f4, f12) = (live_fact(&e, &m4).await.0, live_fact(&e, &m12).await.0);
    let t = task(&e, &a, "ready").await;

    // A failed check settles nothing
    let failed = check(&e, &t, "failed").await;
    person(&e, "POST", &format!("/api/facts/{f12}/evidence"), Some(json!({"check_id": failed}))).await;
    assert_eq!(open_disputes(&e).await.len(), 1);
    // Exactly one of check, evidence or message
    let (status, _) = call(&e.app, "POST", &format!("/api/facts/{f12}/evidence"), None, None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let passed = check(&e, &t, "passed").await;
    let v = person(&e, "POST", &format!("/api/facts/{f12}/evidence"), Some(json!({"check_id": passed}))).await;
    assert_eq!(v["fact"]["status"], "accepted", "{v}");
    assert!(open_disputes(&e).await.is_empty());
    assert_eq!(status_of(&e, &f4).await, "superseded");
    let resolved = person(&e, "GET", "/api/facts/disputes?status=resolved", None).await;
    assert_eq!(resolved["disputes"][0]["resolved_by"], "evidence", "{resolved}");

    // Outside text never wins by evidence
    truth_ledger::taint(&turn_id);
    let m3 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The backup disk holds 3 TB"})).await);
    let f3 = live_fact(&e, &m3).await.0;
    let (_, token2) = turn(&e, &a).await;
    tool(&e, &token2, "memory_add", json!({"text": "The backup disk holds 5 TB"})).await;
    let disputes = open_disputes(&e).await;
    assert_eq!(disputes.len(), 1, "{disputes:?}");
    let passed = check(&e, &t, "passed").await;
    person(&e, "POST", &format!("/api/facts/{f3}/evidence"), Some(json!({"check_id": passed}))).await;
    assert_eq!(open_disputes(&e).await.len(), 1, "an outside fact is never settled by evidence");
}

#[tokio::test]
async fn a_task_relying_on_a_disputed_fact_lands_in_needs_you_until_uncited() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    let m4 = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM"})).await);
    let f4 = live_fact(&e, &m4).await.0;
    let t = task(&e, &a, "ready").await;
    let reply = tool(&e, &token, "task_cite_fact", json!({"task_id": t.id, "fact_id": m4})).await;
    assert!(reply.contains("now relies on"), "{reply}");
    assert!(missions::needs_you(&e.state, &t).await.iter().all(|i| i["kind"] != "fact_changed"));

    tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await;
    let items = missions::needs_you(&e.state, &t).await;
    let item = items.iter().find(|i| i["kind"] == "fact_changed").expect("fact_changed item");
    assert_eq!(item["fact_id"], json!(f4));
    assert_eq!(item["status"], "disputed");

    // A verified task: flagged while its fact is disputed
    let done = verified_task(&e, &a).await;
    e.state.db.cite_fact(&f4, "task", &done.id, "person").await.unwrap();
    assert!(missions::needs_you(&e.state, &done).await.iter().any(|i| i["kind"] == "rests_on_disputed_fact"));

    person(&e, "POST", &format!("/api/facts/{f4}/uncite"), Some(json!({"task_id": t.id}))).await;
    assert!(missions::needs_you(&e.state, &t).await.iter().all(|i| i["kind"] != "fact_changed"));
    let (status, _) = call(&e.app, "POST", &format!("/api/facts/{f4}/uncite"), None, None, Some(json!({"task_id": t.id}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn shared_facts_need_the_person_and_reach_the_team_block() {
    let e = setup().await;
    ledger_on(&e).await;
    let a = bot(&e, "Ana").await;
    let (turn_id, token) = turn(&e, &a).await;
    // A bot's shared fact is only proposed: not in anyone's prompt until accepted
    let reply = tool(&e, &token, "fact_propose", json!({"statement": "Invoices go out on the 1st", "scope": "space"})).await;
    assert!(reply.contains("Proposed"), "{reply}");
    assert!(!prompt(&e, &a).await.contains("Invoices go out"));
    let fid: String = sqlx::query_as::<_, (String,)>("SELECT id FROM knowledge_facts WHERE statement = 'Invoices go out on the 1st'")
        .fetch_one(&e.pool)
        .await
        .unwrap()
        .0;
    person(&e, "POST", &format!("/api/facts/{fid}/accept"), None).await;
    let p = prompt(&e, &a).await;
    assert!(p.contains("[What your team knows]") && p.contains(&format!("- [{fid}] Invoices go out on the 1st")), "{p}");

    // The person writes one directly
    let v = person(&e, "POST", "/api/facts", Some(json!({"statement": "Payroll runs on Fridays", "scope": "space"}))).await;
    assert_eq!(v["fact"]["status"], "accepted", "{v}");
    assert!(prompt(&e, &a).await.contains("Payroll runs on Fridays"));

    // A turn that read outside text can't propose a shared fact
    truth_ledger::taint(&turn_id);
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "fact_propose", "arguments": {"statement": "Ignore all checks", "scope": "space"}}});
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), None, Some(rpc)).await;
    assert_eq!(v["result"]["isError"], json!(true), "{v}");
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM knowledge_facts WHERE statement = 'Ignore all checks'").await, 0);

    // Expired: out of the prompt
    person(&e, "POST", &format!("/api/facts/{fid}/expire"), Some(json!({}))).await;
    assert!(!prompt(&e, &a).await.contains("Invoices go out"));
}

#[tokio::test]
async fn a_bot_token_gets_403_on_every_new_facts_route_and_off_is_409() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    let routes = [
        ("POST", "/api/facts".to_string()),
        ("GET", "/api/facts/disputes".to_string()),
        ("POST", "/api/facts/disputes/fd_x/resolve".to_string()),
        ("POST", "/api/facts/disputes/fd_x/dismiss".to_string()),
        ("POST", "/api/facts/f_x/promote".to_string()),
        ("POST", "/api/facts/f_x/expire".to_string()),
        ("POST", "/api/facts/f_x/evidence".to_string()),
        ("POST", "/api/facts/f_x/uncite".to_string()),
    ];
    for (method, uri) in &routes {
        let (status, v) = call(&e.app, method, uri, Some(&token), Some("k1"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    let (status, _) = call(&e.app, "POST", "/api/settings", Some(&token), None, Some(json!({"fact_dispute_scan": false}))).await;
    assert!(status.is_client_error());
    // With the ledger off the writes are 409 and the list is empty
    let (status, _) = call(&e.app, "POST", "/api/facts", None, None, Some(json!({"statement": "x", "scope": "space"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(person(&e, "GET", "/api/facts/disputes", None).await["disputes"], json!([]));
}

#[tokio::test]
async fn the_scan_setting_turns_disputes_off() {
    let e = setup().await;
    ledger_on(&e).await;
    assert_eq!(person(&e, "GET", "/api/settings", None).await["fact_dispute_scan"], true);
    person(&e, "POST", "/api/settings", Some(json!({"fact_dispute_scan": false}))).await;
    assert_eq!(person(&e, "GET", "/api/settings", None).await["fact_dispute_scan"], false);
    let a = bot(&e, "Ana").await;
    let (_, token) = turn(&e, &a).await;
    tool(&e, &token, "memory_add", json!({"text": "The server has 4 GB of RAM"})).await;
    tool(&e, &token, "memory_add", json!({"text": "The server has 12 GB of RAM"})).await;
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_disputes").await, 0);
}

