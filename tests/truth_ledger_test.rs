//! Truth Ledger C1, shadow mode (Mission Control Phase C1). Triggers mirror every bot memory into
//! `knowledge_facts`; a note from a turn that read outside text, or an import, lands
//! `outside`/`proposed`; only the person reads facts or decides them, and of racing accepts
//! exactly one changes the fact; the bot's memory block is the same with the ledger off or on.
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
        continuity::PinChange,
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        truth_ledger::TRUTH_LEDGER_DOWN,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{memory_block, SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::truth_ledger,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    general: String,
    pool: SqlitePool,
    data: PathBuf,
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
    Env { app, state, general, pool, data, _tmp: tmp }
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

/// (statement, trust, status, source_kind, source_ref, pinned, decided_by) of a memory's facts, oldest first.
type Row = (String, String, String, String, Option<String>, bool, Option<String>);

async fn facts_of(e: &Env, memory_id: &str) -> Vec<Row> {
    sqlx::query_as("SELECT statement, trust, status, source_kind, source_ref, pinned, decided_by FROM knowledge_facts WHERE memory_id = ? ORDER BY rowid")
        .bind(memory_id)
        .fetch_all(&e.pool)
        .await
        .unwrap()
}

async fn live_fact(e: &Env, memory_id: &str) -> (String, String, String) {
    sqlx::query_as("SELECT id, trust, status FROM knowledge_facts WHERE memory_id = ? AND status <> 'superseded'")
        .bind(memory_id)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

async fn events(e: &Env, memory_id: &str) -> Vec<String> {
    sqlx::query_as::<_, (String,)>(
        "SELECT ev.kind FROM fact_events ev JOIN knowledge_facts f ON f.id = ev.fact_id WHERE f.memory_id = ? ORDER BY ev.id",
    )
    .bind(memory_id)
    .fetch_all(&e.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.0)
    .collect()
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(sql).fetch_one(pool).await.unwrap().0
}

/// A memory the bot itself wrote, mirrored as a fact a person can decide: outside, so proposed.
async fn proposed_fact(e: &Env) -> String {
    let a = bot(e, "Ana").await;
    let (turn_id, token) = turn(e, &a).await;
    truth_ledger::taint(&turn_id);
    let id = saved_id(&tool(e, &token, "memory_add", json!({"text": "The vendor's SLA is 99.9%"})).await);
    live_fact(e, &id).await.0
}

#[tokio::test]
async fn the_setting_is_off_by_default_person_only_and_validated() {
    let e = setup().await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!(s["truth_ledger"], "off");
    let (status, _) = call(&e.app, "POST", "/api/settings", None, None, Some(json!({"truth_ledger": "loud"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for m in ["shadow", "on", "off"] {
        ledger(&e, m).await;
        assert_eq!(person(&e, "GET", "/api/settings", None).await["truth_ledger"], m);
    }
    // Off: nothing to read and nothing to decide, though the mirror keeps the facts
    let a = bot(&e, "Ana").await;
    e.state.db.add_memory(&a.id, "Likes tea", "agent").await.unwrap();
    assert_eq!(person(&e, "GET", "/api/facts", None).await, json!({"setting": "off", "facts": []}));
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM knowledge_facts").await, 1);
}

#[tokio::test]
async fn mirror_follows_insert_update_pin_and_delete() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let m = e.state.db.add_memory(&a.id, "Standup is at 10", "agent").await.unwrap();
    assert_eq!(facts_of(&e, &m.id).await, vec![("Standup is at 10".into(), "bot".into(), "accepted".into(), "bot".into(), None, false, None)]);
    let (scope, scope_id, author, space): (String, String, Option<String>, String) =
        sqlx::query_as("SELECT scope, scope_id, author_kwant_id, space_id FROM knowledge_facts WHERE memory_id = ?").bind(&m.id).fetch_one(&e.pool).await.unwrap();
    assert_eq!((scope.as_str(), scope_id.as_str(), author.as_deref(), space.as_str()), ("bot", a.id.as_str(), Some(a.id.as_str()), "spc_default"));

    // Update: the old fact is superseded by the new one, history kept
    e.state.db.agent_update_memory(&a.id, &m.id, "Standup is at 9:30").await.unwrap();
    let rows = facts_of(&e, &m.id).await;
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].0.as_str(), rows[0].2.as_str()), ("Standup is at 10", "superseded"));
    assert_eq!((rows[1].0.as_str(), rows[1].2.as_str()), ("Standup is at 9:30", "accepted"));
    let (old_next, new_id): (Option<String>, String) = sqlx::query_as(
        "SELECT (SELECT superseded_by FROM knowledge_facts WHERE memory_id = ?1 AND status = 'superseded'), \
                (SELECT id FROM knowledge_facts WHERE memory_id = ?1 AND status = 'accepted')",
    )
    .bind(&m.id)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(old_next.as_deref(), Some(new_id.as_str()));

    // The person edits it: their words now
    e.state.db.update_memory(&a.id, &m.id, "Standup is at 9").await.unwrap();
    let rows = facts_of(&e, &m.id).await;
    assert_eq!((rows[2].1.as_str(), rows[2].2.as_str(), rows[2].3.as_str(), rows[2].6.as_deref()), ("person", "accepted", "person", Some("person")));

    // Pin and unpin
    e.state.db.set_memory_pinned(&a.id, &m.id, PinChange { pinned: true, by_person: true }).await.unwrap();
    assert!(facts_of(&e, &m.id).await[2].5);
    e.state.db.set_memory_pinned(&a.id, &m.id, PinChange { pinned: false, by_person: true }).await.unwrap();
    assert!(!facts_of(&e, &m.id).await[2].5);

    // Delete: the fact is superseded with no successor, nothing live is left
    assert!(e.state.db.delete_memory(&a.id, &m.id).await.unwrap());
    assert!(facts_of(&e, &m.id).await.iter().all(|r| r.2 == "superseded"));
    assert_eq!(events(&e, &m.id).await, ["created", "superseded", "created", "superseded", "created", "pinned", "unpinned", "removed"]);

    // Events are append-only
    let upd = sqlx::query("UPDATE fact_events SET kind = 'created'").execute(&e.pool).await;
    assert!(upd.is_err(), "fact_events can't be rewritten");
    let del = sqlx::query("DELETE FROM fact_events").execute(&e.pool).await;
    assert!(del.is_err(), "fact_events of a fact that exists can't be deleted");
}

#[tokio::test]
async fn status_rules_hold_in_the_database() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let m = e.state.db.add_memory(&a.id, "Likes tea", "agent").await.unwrap();
    let (fid, _, _) = live_fact(&e, &m.id).await;
    // Outside can't be accepted except by the person
    let r = sqlx::query("UPDATE knowledge_facts SET trust = 'outside', status = 'accepted', decided_by = 'evidence' WHERE id = ?").bind(&fid).execute(&e.pool).await;
    assert!(r.is_err(), "outside+accepted needs the person");
    // A superseded fact is final
    sqlx::query("UPDATE knowledge_facts SET status = 'superseded' WHERE id = ?").bind(&fid).execute(&e.pool).await.unwrap();
    let r = sqlx::query("UPDATE knowledge_facts SET status = 'accepted' WHERE id = ?").bind(&fid).execute(&e.pool).await;
    assert!(r.is_err(), "superseded is terminal");
    // A team-scope fact can't be accepted except by the person
    let r = sqlx::query(
        "INSERT INTO knowledge_facts (id, space_id, scope, scope_id, statement, source_kind, trust, status, valid_from, created_at, updated_at) \
         VALUES ('f_x', 'spc_default', 'team', 't1', 's', 'bot', 'bot', 'accepted', 'now', 'now', 'now')",
    )
    .execute(&e.pool)
    .await;
    assert!(r.is_err(), "non-bot scope needs the person to accept");
}

#[tokio::test]
async fn backfill_is_idempotent() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    for t in ["One", "Two", "Three"] {
        e.state.db.add_memory(&a.id, t, "agent").await.unwrap();
    }
    assert_eq!(e.state.db.backfill_truth_ledger().await.unwrap(), 0, "the mirror already has them");
    sqlx::query("DELETE FROM knowledge_facts").execute(&e.pool).await.unwrap();
    assert_eq!(e.state.db.backfill_truth_ledger().await.unwrap(), 3);
    assert_eq!(e.state.db.backfill_truth_ledger().await.unwrap(), 0, "running it twice changes nothing");
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM knowledge_facts").await, 3);
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_events WHERE kind = 'backfilled'").await, 3);
}

#[tokio::test]
async fn memory_add_in_a_tainted_turn_lands_outside_proposed_and_stays_visible() {
    let e = setup().await;
    ledger(&e, "shadow").await;
    let a = bot(&e, "Ana").await;

    // A clean turn: the bot's own note
    let (_, clean) = turn(&e, &a).await;
    let mine = saved_id(&tool(&e, &clean, "memory_add", json!({"text": "Oleks prefers short replies"})).await);
    assert_eq!(live_fact(&e, &mine).await.1, "bot");
    assert_eq!(live_fact(&e, &mine).await.2, "accepted");

    // A turn that read a web page
    let (turn_id, token) = turn(&e, &a).await;
    assert!(truth_ledger::reads_outside("WebFetch") && truth_ledger::reads_outside("mcp__github__get_issue"));
    assert!(!truth_ledger::reads_outside("mcp__kwnt_tools__memory_add"));
    truth_ledger::taint(&turn_id);
    let theirs = saved_id(&tool(&e, &token, "memory_add", json!({"text": "The API limit is 50 rps"})).await);
    let rows = facts_of(&e, &theirs).await;
    assert_eq!(rows, vec![("The API limit is 50 rps".into(), "outside".into(), "proposed".into(), "bot".into(), Some(turn_id.clone()), false, None)]);
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_provenance").await, 0, "provenance is consumed");

    // Still visible to the bot (D3)
    let memories = e.state.db.list_memories(&a.id).await.unwrap();
    assert!(memory_block(&memories, true).contains("The API limit is 50 rps"));

    // Rewriting it in a later clean turn doesn't launder it
    let (_, later) = turn(&e, &a).await;
    tool(&e, &later, "memory_update", json!({"id": theirs, "text": "The API limit is 60 rps"})).await;
    assert_eq!(live_fact(&e, &theirs).await.1, "outside");
    assert_eq!(live_fact(&e, &theirs).await.2, "proposed");

    // The turn's end clears the bit
    truth_ledger::end_turn(&turn_id);
    assert!(!truth_ledger::is_tainted(&turn_id));
}

#[tokio::test]
async fn an_import_lands_outside_proposed_with_source_still_user() {
    let e = setup().await;
    let fixture = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import/kwant-bundle.json")).unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/api/import/preview?space=spc_default&filename=kwant-bundle.json")
        .header("content-type", "application/octet-stream")
        .body(Body::from(fixture))
        .unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let preview: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let (status, applied) = call(&e.app, "POST", "/api/import/apply?space=spc_default", None, None, Some(json!({"bundle": preview["bundle"], "skip": []}))).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let import_id = applied["import"]["id"].as_str().unwrap();
    let bot_id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap();
    let memories = e.state.db.list_memories(bot_id).await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!((memories[0].source.as_str(), memories[0].imported), ("user", true), "the memory row is unchanged");
    let rows = facts_of(&e, &memories[0].id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_str(), rows[0].3.as_str(), rows[0].4.as_deref()), ("outside", "proposed", "import", Some(import_id)));

    // Undoing the import takes its facts with it
    let token = e.state.api_token.reveal().to_string();
    let (status, v) = call(&e.app, "POST", &format!("/api/imports/{import_id}/undo"), Some(&token), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(count(&e.pool, &format!("SELECT COUNT(*) FROM knowledge_facts WHERE scope_id = '{bot_id}'")).await, 0);
}

#[tokio::test]
async fn memory_block_is_byte_identical_with_the_ledger_off_and_shadow() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let (turn_id, token) = turn(&e, &a).await;
    e.state.db.add_memory(&a.id, "Person wrote this", "user").await.unwrap();
    tool(&e, &token, "memory_add", json!({"text": "Own note", "pinned": true})).await;
    truth_ledger::taint(&turn_id);
    tool(&e, &token, "memory_add", json!({"text": "Read on a web page"})).await;

    let mut blocks = Vec::new();
    for m in ["off", "shadow", "on"] {
        ledger(&e, m).await;
        let memories = e.state.db.list_memories(&a.id).await.unwrap();
        blocks.push((memory_block(&memories, true), memory_block(&memories, false)));
    }
    assert_eq!(blocks[0], blocks[1]);
    assert_eq!(blocks[0], blocks[2], "`on` is shadow in C1");
}

#[tokio::test]
async fn a_bot_token_gets_403_on_every_facts_route() {
    let e = setup().await;
    ledger(&e, "shadow").await;
    let fid = proposed_fact(&e).await;
    let b = bot(&e, "Bea").await;
    let (_, token) = turn(&e, &b).await;
    for (method, uri) in [
        ("GET", "/api/facts".to_string()),
        ("GET", format!("/api/facts/{fid}")),
        ("POST", format!("/api/facts/{fid}/accept")),
        ("POST", format!("/api/facts/{fid}/reject")),
    ] {
        let (status, v) = call(&e.app, method, &uri, Some(&token), Some("k1"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    // Nor can a bot turn the ledger on or off
    let (status, _) = call(&e.app, "POST", "/api/settings", Some(&token), None, Some(json!({"truth_ledger": "off"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(live_fact(&e, &sqlx::query_as::<_, (String,)>("SELECT memory_id FROM knowledge_facts WHERE id = ?").bind(&fid).fetch_one(&e.pool).await.unwrap().0).await.2, "proposed");
}

#[tokio::test]
async fn the_person_reads_and_decides_facts() {
    let e = setup().await;
    let fid = proposed_fact(&e).await;
    let (status, _) = call(&e.app, "POST", &format!("/api/facts/{fid}/accept"), None, None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing to decide with the ledger off");
    ledger(&e, "shadow").await;

    let list = person(&e, "GET", "/api/facts?status=proposed", None).await;
    assert_eq!(list["setting"], "shadow");
    assert_eq!(list["facts"].as_array().unwrap().len(), 1);
    assert_eq!(person(&e, "GET", "/api/facts?q=SLA", None).await["facts"].as_array().unwrap().len(), 1);
    assert_eq!(person(&e, "GET", "/api/facts?q=nothing%25", None).await["facts"].as_array().unwrap().len(), 0);
    let (status, _) = call(&e.app, "GET", "/api/facts?status=maybe", None, None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let one = person(&e, "GET", &format!("/api/facts/{fid}"), None).await;
    assert_eq!(one["fact"]["status"], "proposed");
    assert_eq!(one["events"][0]["kind"], "created");
    let (status, _) = call(&e.app, "GET", "/api/facts/f_nope", None, None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let v = person(&e, "POST", &format!("/api/facts/{fid}/accept"), None).await;
    assert_eq!((v["fact"]["status"].as_str(), v["fact"]["decided_by"].as_str()), (Some("accepted"), Some("person")));
    let (status, v) = call(&e.app, "POST", &format!("/api/facts/{fid}/reject"), None, None, None).await;
    assert_eq!((status, v["status"].as_str()), (StatusCode::CONFLICT, Some("accepted")));

    // Reject: superseded, the note itself untouched in C1
    let a = bot(&e, "Cy").await;
    let (turn_id, token) = turn(&e, &a).await;
    truth_ledger::taint(&turn_id);
    let mid = saved_id(&tool(&e, &token, "memory_add", json!({"text": "Prices rise in May"})).await);
    let fid2 = live_fact(&e, &mid).await.0;
    let v = person(&e, "POST", &format!("/api/facts/{fid2}/reject"), None).await;
    assert_eq!(v["fact"]["status"], "superseded");
    assert_eq!(v["events"].as_array().unwrap().last().unwrap()["kind"], "rejected");
    assert_eq!(e.state.db.list_memories(&a.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn ten_racing_accepts_make_one_change() {
    let e = setup().await;
    ledger(&e, "shadow").await;
    let fid = proposed_fact(&e).await;
    let mut handles = Vec::new();
    for i in 0..10 {
        let app = e.app.clone();
        let uri = format!("/api/facts/{fid}/accept");
        handles.push(tokio::spawn(async move { call(&app, "POST", &uri, None, Some(&format!("race-{i}")), None).await.0 }));
    }
    let mut statuses = Vec::new();
    for h in handles {
        statuses.push(h.await.unwrap());
    }
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1, "{statuses:?}");
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::CONFLICT).count(), 9, "{statuses:?}");
    assert_eq!(count(&e.pool, &format!("SELECT COUNT(*) FROM fact_events WHERE fact_id = '{fid}' AND kind = 'accepted'")).await, 1);

    // The same key replays rather than conflicting
    let (s1, v1) = call(&e.app, "POST", &format!("/api/facts/{fid}/accept"), None, Some("race-again"), None).await;
    let (s2, v2) = call(&e.app, "POST", &format!("/api/facts/{fid}/accept"), None, Some("race-again"), None).await;
    assert_eq!((s1, s2), (StatusCode::CONFLICT, StatusCode::CONFLICT));
    assert_eq!(v1, v2);
}

#[tokio::test]
async fn deleting_a_bot_removes_its_facts() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    e.state.db.add_memory(&a.id, "Secret-ish preference", "agent").await.unwrap();
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM knowledge_facts").await, 1);
    let (status, v) = call(&e.app, "DELETE", &format!("/api/kwants/{}", a.id), None, None, None).await;
    assert!(status.is_success(), "{status} {v}");
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM knowledge_facts").await, 0);
    assert_eq!(count(&e.pool, "SELECT COUNT(*) FROM fact_events").await, 0);
}

#[tokio::test]
async fn migrating_a_copy_of_an_existing_database_keeps_every_memory() {
    // An existing install: memories written before the ledger existed
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let b = bot(&e, "Bea").await;
    for i in 0..5 {
        e.state.db.add_memory(&a.id, &format!("Ana note {i}"), "agent").await.unwrap();
    }
    e.state.db.add_memory(&b.id, "Bea's person note", "user").await.unwrap();
    for stmt in TRUTH_LEDGER_DOWN {
        sqlx::query(stmt).execute(&e.pool).await.unwrap();
    }
    sqlx::query("UPDATE kwant_memories SET imported = 1 WHERE kwant_id = ?").bind(&b.id).execute(&e.pool).await.unwrap();
    let memories_before = count(&e.pool, "SELECT COUNT(*) FROM kwant_memories").await;
    let before: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, text, source FROM kwant_memories ORDER BY id").fetch_all(&e.pool).await.unwrap();
    e.pool.close().await;

    // Copy it and start the new binary on the copy
    let copy = tempdir().unwrap();
    let data = copy.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    for f in ["kwnt.db", "kwnt.db-wal", "kwnt.db-shm"] {
        if e.data.join(f).exists() {
            std::fs::copy(e.data.join(f), data.join(f)).unwrap();
        }
    }
    let (_, state, pool) = open(&data).await;
    let after: Vec<(String, String, String)> = sqlx::query_as("SELECT id, text, source FROM kwant_memories ORDER BY id").fetch_all(&pool).await.unwrap();
    assert_eq!(after, before, "no memory moved or changed");
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM knowledge_facts").await, memories_before);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM knowledge_facts f JOIN kwant_memories m ON m.id = f.memory_id AND m.text = f.statement").await, memories_before);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM knowledge_facts WHERE trust = 'outside' AND status = 'proposed'").await, 1, "the imported note");
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM fact_events WHERE kind = 'backfilled'").await, memories_before);
    // Starting again changes nothing
    pool.close().await;
    drop(state);
    let (_, _state, pool) = open(&data).await;
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM knowledge_facts").await, memories_before);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM fact_events").await, memories_before);
}
