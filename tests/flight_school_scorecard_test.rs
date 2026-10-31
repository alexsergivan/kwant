//! Mission Control Phase C4: Flight School scorecards and tier proposals. The contract's
//! acceptance tests (docs/mission-control.md §14, "Scorecards and proposals"): scorecard counts on
//! seeded rows, `tier_propose` refused when the bot doesn't qualify and accepted when it does, a
//! proposal going stale when the card drops or the tier moves, grant writing exactly one live
//! policy with racing grants giving one 200, only the person deciding, and the payment floor
//! still carding after a grant to act.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration as Days, Utc};
use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const STRIPE_POST: &str = "curl -X POST https://api.stripe.com/v1/charges -u sk_test_x: -d amount=100 -d currency=usd";

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
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
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app: create_router(state.clone()), bot, general, state, _tmp: tmp };
    person(&e, "POST", "/api/settings", json!({"mission_board": true})).await;
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

async fn person(e: &Env, method: &str, uri: &str, body: Value) -> Value {
    let body = (method != "GET").then_some(body);
    let (status, v) = call(&e.app, method, uri, Some(e.state.api_token.reveal()), body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn set_tier(e: &Env, class: &str, tier: &str) {
    person(e, "POST", "/api/capabilities", json!({"kwant_id": e.bot.id, "task_class": class, "tier": tier})).await;
}

async fn flight_school(e: &Env, on: bool) {
    person(e, "POST", "/api/settings", json!({"flight_school": on})).await;
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// A live turn of the bot in #general with its own tools, working in `folder`; returns (turn id, token).
async fn turn(e: &Env, folder: &Path) -> (String, String) {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let (_, token) =
        mcp_runtime::prepare_turn_with_opts(&e.state, &e.bot, Some(turn_id.clone()), Some(folder.to_path_buf()), Some(scope), Default::default())
            .await
            .unwrap();
    e.state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: e.bot.id.clone(),
        context_id: e.general.clone(),
        thread_id: None,
        state: "running".into(),
        since: Utc::now(),
        purpose: None,
    });
    (turn_id, token)
}

async fn end(e: &Env, turn_id: &str, token: &str) {
    mcp_runtime::finish_turn(&e.state, token).await;
    e.state.turns.end_turn(turn_id);
}

async fn self_tool(e: &Env, token: &str, name: &str, args: Value) -> (bool, String) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (v["result"]["isError"].as_bool().unwrap_or(false), v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
}

/// A person's mission with one task the bot owns, of `class`, claimed by the bot (with a
async fn claimed_task(e: &Env, class: &str, checkpoint: bool) -> (String, PathBuf) {
    person(e, "POST", "/api/settings", json!({"checkpoints": checkpoint})).await;
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": e.bot.id, "budget_tokens": 900_000,
                      "channel_id": e.general, "tasks": [{"title": "A", "owner": e.bot.id, "task_class": class}]});
    let m = person(e, "POST", "/api/missions", body).await;
    let detail = person(e, "GET", &format!("/api/missions/{}", id(&m)), json!(null)).await;
    let tid = id(&detail["tasks"][0]);
    let mission = e.state.db.get_mission(&id(&m)).await.unwrap().unwrap();
    let dir = PathBuf::from(mission.workspace_path);
    std::fs::create_dir_all(&dir).unwrap();
    let (turn_id, token) = turn(e, &dir).await;
    let (err, text) = self_tool(e, &token, "task_claim", json!({"task_id": tid})).await;
    assert!(!err, "{text}");
    end(e, &turn_id, &token).await;
    assert_eq!(e.state.db.run_has_checkpoint(&tid).await.unwrap(), checkpoint);
    (tid, dir)
}

/// What claude's permission prompt gets for `tool {input}` in a turn working in `folder`:
/// `Ok(behavior)` when kwnt answers without a card, `Err(card details)` when it shows one (denied).
async fn claude_asks(e: &Env, folder: &Path, tool: &str, input: Value) -> Result<String, Value> {
    let (turn_id, token) = turn(e, folder).await;
    let out = ask_in(e, &token, tool, input).await;
    end(e, &turn_id, &token).await;
    out
}

async fn ask_in(e: &Env, token: &str, tool: &str, input: Value) -> Result<String, Value> {
    let mut events = e.state.event_sender.subscribe();
    let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
    let app = e.app.clone();
    let path = format!("/api/mcp/permission/{token}");
    let mut prompt = tokio::spawn(async move { call(&app, "POST", &path, None, Some(ask)).await });
    let behavior = |v: &Value| {
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["behavior"].as_str().unwrap().to_string()
    };
    let answered = tokio::time::timeout(Duration::from_millis(1500), &mut prompt).await;
    let card = loop {
        match events.try_recv() {
            Ok(RuntimeEvent::ApprovalRequired { id, details, .. }) => break Some((id, details)),
            Ok(_) => continue,
            Err(_) => break None,
        }
    };
    match (answered, card) {
        (Ok(done), None) => Ok(behavior(&done.unwrap().1)),
        (Err(_), Some((id, details))) => {
            assert!(e.state.broker.resolve(&id, ApprovalDecision::Deny).await);
            let (_, v) = prompt.await.unwrap();
            assert_eq!(behavior(&v), "deny");
            Err(details)
        }
        (answered, card) => panic!("answered {} with card {card:?}", answered.is_ok()),
    }
}

/// One settled task kwant would have written: owned by the bot, of `class`, claimed once, in
/// `state` (verified through a passing person check), finished `days_ago`.
#[derive(Clone)]
struct Seed {
    class: &'static str,
    state: &'static str,
    attempts: i64,
    days_ago: i64,
    claimed: bool,
    denied_call: bool,
    denied_failure: bool,
    check_failure: bool,
    approved_card: bool,
    external_call: bool,
    flag: Option<&'static str>,
    person_retry: bool,
    spent: i64,
    recovery: i64,
}

fn verified() -> Seed {
    Seed {
        class: "research",
        state: "verified",
        attempts: 1,
        days_ago: 1,
        claimed: true,
        denied_call: false,
        denied_failure: false,
        check_failure: false,
        approved_card: false,
        external_call: false,
        flag: None,
        person_retry: false,
        spent: 100,
        recovery: 0,
    }
}

/// Seeds tasks straight into the database, oldest first (so the last one is the newest), at
/// one-minute steps so their order is fixed.
async fn seed(e: &Env, tasks: Vec<Seed>) -> Vec<String> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", e.state.config.data_dir.join("kwnt.db").display()))
        .await
        .unwrap();
    let mut ids = Vec::new();
    let base = Utc::now() - Days::minutes(tasks.len() as i64 + 5);
    for (i, s) in tasks.iter().enumerate() {
        let id = uuid::Uuid::new_v4().to_string();
        let finished: DateTime<Utc> = base + Days::minutes(i as i64) - Days::days(s.days_ago);
        let first = if s.state == "verified" { "review" } else { s.state };
        sqlx::query(
            "INSERT INTO tasks (id, space_id, title, state, owner_kwant_id, budget_tokens, spent_tokens, attempts, source_kind, created_by, \
             created_at, updated_at, task_class, recovery_spent_tokens, finished_at) \
             VALUES (?, 'spc_default', 'seeded', ?, ?, 100000, ?, ?, 'person', 'person', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(first)
        .bind(&e.bot.id)
        .bind(s.spent)
        .bind(s.attempts)
        .bind(finished)
        .bind(finished)
        .bind(s.class)
        .bind(s.recovery)
        .bind(finished)
        .execute(&pool)
        .await
        .unwrap();
        if s.state == "verified" {
            sqlx::query(
                "INSERT INTO outcome_checks (id, task_id, kind, spec, status, checked_by, requested_by, created_at) \
                 VALUES (?, ?, 'person', '{}', 'passed', 'person', 'person', ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&id)
            .bind(finished)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("UPDATE tasks SET state = 'verified' WHERE id = ?").bind(&id).execute(&pool).await.unwrap();
        }
        if s.claimed {
            let ms = finished.timestamp_millis();
            sqlx::query("INSERT INTO agent_claims (id, task_id, kwant_id, claimed_at, expires_at, released_at, release_reason) VALUES (?, ?, ?, ?, ?, ?, 'done')")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&id)
                .bind(&e.bot.id)
                .bind(ms - 60_000)
                .bind(ms + 60_000)
                .bind(ms)
                .execute(&pool)
                .await
                .unwrap();
        }
        let call = |effect: &'static str, decision: &'static str, card: bool| {
            sqlx::query(
                "INSERT INTO turn_tool_calls (turn_id, kwant_id, task_id, tool_name, effect, summary, decision, approval_id, created_at) \
                 VALUES ('t', ?, ?, 'Bash', ?, 'x', ?, ?, ?)",
            )
            .bind(e.bot.id.clone())
            .bind(id.clone())
            .bind(effect)
            .bind(decision)
            .bind(card.then(|| uuid::Uuid::new_v4().to_string()))
            .bind(finished)
        };
        if s.denied_call {
            call("external", "denied", true).execute(&pool).await.unwrap();
        }
        if s.approved_card {
            call("workspace", "approved", true).execute(&pool).await.unwrap();
        }
        if s.external_call {
            call("external", "allowed", false).execute(&pool).await.unwrap();
        }
        for (on, class) in [(s.denied_failure, "denied"), (s.check_failure, "check_failed")] {
            if on {
                sqlx::query("INSERT INTO task_failures (id, task_id, attempt, source, class, summary, created_at) VALUES (?, ?, 1, 'check', ?, 'x', ?)")
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(&id)
                    .bind(class)
                    .bind(finished)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        if let Some(resolution) = s.flag {
            let resolved = (resolution != "open").then_some(finished);
            sqlx::query(
                "INSERT INTO task_flags (id, task_id, kind, cause, cause_task_id, created_at, resolved_at, resolved_by, resolution) \
                 VALUES (?, ?, 'may_be_invalid', 'evidence_changed', ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&id)
            .bind(&id)
            .bind(finished)
            .bind(resolved)
            .bind(resolved.map(|_| "person"))
            .bind((resolution != "open").then_some(resolution))
            .execute(&pool)
            .await
            .unwrap();
        }
        if s.person_retry {
            sqlx::query("INSERT INTO task_events (task_id, kind, actor, from_state, to_state, created_at) VALUES (?, 'state', 'person', 'failed', 'ready', ?)")
                .bind(&id)
                .bind(finished)
                .execute(&pool)
                .await
                .unwrap();
        }
        ids.push(id);
    }
    pool.close().await;
    ids
}

async fn card(e: &Env, class: &str) -> Value {
    person(e, "GET", &format!("/api/capabilities/{}/scorecard?class={class}", e.bot.id), json!(null)).await
}

/// The bot calls `tier_propose` in a chat turn; (refused?, text).
async fn propose(e: &Env, class: &str) -> (bool, String) {
    let dir = e.state.config.data_dir.join("chat");
    std::fs::create_dir_all(&dir).unwrap();
    let (turn_id, token) = turn(e, &dir).await;
    let out = self_tool(e, &token, "tier_propose", json!({"task_class": class, "reason": "I keep landing these"})).await;
    end(e, &turn_id, &token).await;
    out
}

async fn proposals(e: &Env, status: &str) -> Vec<Value> {
    let v = person(e, "GET", &format!("/api/capabilities/proposals?status={status}"), json!(null)).await;
    v["proposals"].as_array().unwrap().clone()
}

async fn live_rows(e: &Env, class: &str) -> Vec<Value> {
    let v = person(e, "GET", &format!("/api/capabilities?kwant_id={}", e.bot.id), json!(null)).await;
    v["policies"].as_array().unwrap().iter().filter(|p| p["task_class"] == class).cloned().collect()
}

#[tokio::test]
async fn the_scorecard_counts_seeded_rows() {
    let e = setup().await;
    let mut tasks = vec![
        // 3 verified first try, one with an approved card and one external call
        verified(),
        Seed { approved_card: true, external_call: true, ..verified() },
        verified(),
        // verified on the second attempt, after a failed check, with a person retry
        Seed { attempts: 2, check_failure: true, person_retry: true, spent: 300, recovery: 50, ..verified() },
        // verified, then its evidence changed and the person said redo: a false success
        Seed { flag: Some("redo"), ..verified() },
        // verified and flagged, but the person said it still holds: not a false success
        Seed { flag: Some("still_valid"), ..verified() },
        // failed with a denied call (and a denied failure)
        Seed { state: "failed", denied_call: true, denied_failure: true, ..verified() },
        Seed { state: "cancelled", ..verified() },
    ];
    // Not counted: another class, older than 30 days, never claimed
    tasks.push(Seed { class: "general", ..verified() });
    tasks.push(Seed { days_ago: 31, ..verified() });
    tasks.push(Seed { claimed: false, ..verified() });
    seed(&e, tasks).await;
    let c = card(&e, "research").await;
    assert_eq!(c["tasks"], json!({"settled": 8, "verified": 6, "failed": 1, "cancelled": 1}), "{c}");
    assert_eq!(c["first_try"], json!({"k": 5, "n": 6, "rate": 0.833}), "{c}");
    assert_eq!(c["false_success"]["k"], 1, "{c}");
    assert_eq!(c["failures"], json!({"total": 2, "by_class": {"denied": 1, "check_failed": 1}}), "{c}");
    assert_eq!(c["cards"]["shown"], 2, "{c}");
    assert_eq!(c["cards"]["approved"], 1, "{c}");
    assert_eq!(c["cards"]["denied"], 1, "{c}");
    assert_eq!(c["external_calls"], 2, "{c}");
    assert_eq!(c["interventions"], 1, "{c}");
    assert_eq!(c["tokens"], json!({"spent": 1000, "recovery": 50}), "{c}");
    assert_eq!(c["window_days"], 30);
    // No policy row is act: nothing to ask for
    assert_eq!(c["tier"], "act");
    assert_eq!(c["next_tier"], Value::Null);
    assert_eq!(c["eligible"], false);
    // A class with no tasks: no rate without its n
    let empty = card(&e, "nothing-here").await;
    assert_eq!(empty["first_try"], json!({"k": 0, "n": 0, "rate": null}), "{empty}");
    // At sandbox the card says what act still needs
    set_tier(&e, "research", "sandbox").await;
    let c = card(&e, "research").await;
    assert_eq!(c["next_tier"], "act");
    let missing = c["missing"].to_string();
    assert!(missing.contains("4 more verified") && missing.contains("denied") && missing.contains("false success"), "{missing}");
    assert!(!missing.contains("first-try"), "5 of 6 is over 70%: {missing}");
    // A bot reads scorecards; class slugs are checked
    let (s, _) = call(&e.app, "GET", &format!("/api/capabilities/{}/scorecard?class=Bad%20Class", e.bot.id), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn propose_is_refused_until_the_bot_qualifies_then_waits_for_the_person() {
    let e = setup().await;
    // Off: no tiers to ask for
    let (err, text) = propose(&e, "research").await;
    assert!(err && text.contains("Flight School is off"), "{text}");
    flight_school(&e, true).await;
    // Act (no row): nothing above it
    let (err, text) = propose(&e, "research").await;
    assert!(err && text.contains("already have act"), "{text}");
    set_tier(&e, "*", "observe").await;
    seed(&e, vec![verified(); 4]).await;
    let (err, text) = propose(&e, "research").await;
    assert!(err && text.contains("1 more verified"), "{text}");
    assert!(proposals(&e, "proposed").await.is_empty());
    seed(&e, vec![verified()]).await;
    let (err, text) = propose(&e, "research").await;
    assert!(!err, "{text}");
    let open = proposals(&e, "proposed").await;
    assert_eq!(open.len(), 1);
    let p = &open[0];
    assert_eq!((p["from_tier"].as_str(), p["to_tier"].as_str(), p["task_class"].as_str()), (Some("observe"), Some("sandbox"), Some("research")));
    assert_eq!(p["kind"], "tier_proposal");
    assert_eq!(p["scorecard"]["tasks"]["verified"], 5, "the snapshot is kwant's card: {p}");
    assert_eq!(p["reason"], "I keep landing these");
    // Asking again reports the one waiting, it doesn't add another
    let (err, text) = propose(&e, "research").await;
    assert!(!err && text.contains("already asked"), "{text}");
    assert_eq!(proposals(&e, "proposed").await.len(), 1);
    // Proposing changes no tier
    let caps = person(&e, "GET", &format!("/api/capabilities?kwant_id={}", e.bot.id), json!(null)).await;
    assert!(caps["effective"].as_array().unwrap().iter().any(|x| x["task_class"] == "research" && x["tier"] == "observe"), "{caps}");
    assert_eq!(caps["proposals"].as_array().unwrap().len(), 1, "{caps}");
    // A denied call in the last 5 tasks refuses the next class's ask
    seed(&e, vec![Seed { class: "ops", ..verified() }; 5]).await;
    seed(&e, vec![Seed { class: "ops", state: "failed", denied_call: true, ..verified() }]).await;
    let (err, text) = propose(&e, "ops").await;
    assert!(err && text.contains("no denied call in the last 5"), "{text}");
}

#[tokio::test]
async fn only_the_person_grants_or_declines_and_a_decline_cools_down() {
    let e = setup().await;
    flight_school(&e, true).await;
    set_tier(&e, "research", "observe").await;
    seed(&e, vec![verified(); 5]).await;
    assert!(!propose(&e, "research").await.0);
    let pid = proposals(&e, "proposed").await[0]["id"].as_str().unwrap().to_string();
    let dir = e.state.config.data_dir.join("chat");
    let (turn_id, token) = turn(&e, &dir).await;
    for verb in ["grant", "decline"] {
        let (s, v) = call(&e.app, "POST", &format!("/api/capabilities/proposals/{pid}/{verb}"), Some(&token), Some(json!({}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "a bot token can't {verb}: {v}");
    }
    let (s, _) = call(&e.app, "GET", "/api/capabilities/proposals", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "reads stay open");
    end(&e, &turn_id, &token).await;
    let v = person(&e, "POST", &format!("/api/capabilities/proposals/{pid}/decline"), json!({"note": "not this week"})).await;
    assert_eq!(v["proposal"]["status"], "declined");
    assert_eq!(v["proposal"]["note"], "not this week");
    assert_eq!(live_rows(&e, "research").await[0]["tier"], "observe", "a decline changes no tier");
    let (s, v) = call(&e.app, "POST", &format!("/api/capabilities/proposals/{pid}/grant"), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["status"], "declined", "{v}");
    let (err, text) = propose(&e, "research").await;
    assert!(err && text.contains("declined"), "{text}");
    // An unknown proposal is 404
    let (s, _) = call(&e.app, "POST", "/api/capabilities/proposals/nope/grant", Some(e.state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_proposal_goes_stale_when_the_card_drops_or_the_tier_moves() {
    let e = setup().await;
    flight_school(&e, true).await;
    set_tier(&e, "research", "observe").await;
    seed(&e, vec![verified(); 5]).await;
    assert!(!propose(&e, "research").await.0);
    // A denied call lands in the last 5 before the person answers
    seed(&e, vec![Seed { state: "failed", denied_call: true, ..verified() }]).await;
    let stale = proposals(&e, "stale").await;
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert!(stale[0]["note"].as_str().unwrap().contains("no longer qualifies"), "{}", stale[0]);
    let pid = stale[0]["id"].as_str().unwrap();
    let (s, v) = call(&e.app, "POST", &format!("/api/capabilities/proposals/{pid}/grant"), Some(e.state.api_token.reveal()), None).await;
    assert_eq!((s, v["status"].as_str()), (StatusCode::CONFLICT, Some("stale")), "{v}");
    assert_eq!(live_rows(&e, "research").await[0]["tier"], "observe");

    // The person moves the tier by hand while a proposal waits: the grant refuses it as stale
    seed(&e, vec![Seed { class: "ops", ..verified() }; 5]).await;
    set_tier(&e, "ops", "observe").await;
    assert!(!propose(&e, "ops").await.0);
    let pid = proposals(&e, "proposed").await[0]["id"].as_str().unwrap().to_string();
    set_tier(&e, "ops", "act").await;
    let (s, v) = call(&e.app, "POST", &format!("/api/capabilities/proposals/{pid}/grant"), Some(e.state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("tier changed"), "{v}");
    assert_eq!(live_rows(&e, "ops").await[0]["tier"], "act", "the stale grant wrote nothing");
}

#[tokio::test]
async fn racing_grants_write_exactly_one_live_policy() {
    let e = setup().await;
    flight_school(&e, true).await;
    set_tier(&e, "research", "observe").await;
    seed(&e, vec![verified(); 5]).await;
    assert!(!propose(&e, "research").await.0);
    let pid = proposals(&e, "proposed").await[0]["id"].as_str().unwrap().to_string();
    let mut events = e.state.event_sender.subscribe();
    let mut racers = Vec::new();
    for _ in 0..10 {
        let app = e.app.clone();
        let uri = format!("/api/capabilities/proposals/{pid}/grant");
        let token = e.state.api_token.reveal().to_string();
        racers.push(tokio::spawn(async move { call(&app, "POST", &uri, Some(&token), None).await }));
    }
    let mut ok = 0;
    for r in racers {
        let (s, v) = r.await.unwrap();
        match s {
            StatusCode::OK => ok += 1,
            StatusCode::CONFLICT => assert_eq!(v["status"], "granted", "{v}"),
            other => panic!("{other}: {v}"),
        }
    }
    assert_eq!(ok, 1, "one grant wins");
    let live = live_rows(&e, "research").await;
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(live[0]["tier"], "sandbox");
    assert_eq!(live[0]["granted_by"], "person");
    let caps = person(&e, "GET", &format!("/api/capabilities?kwant_id={}", e.bot.id), json!(null)).await;
    let grants = caps["events"].as_array().unwrap().iter().filter(|x| x["kind"] == "granted" && x["task_class"] == "research" && x["tier"] == "sandbox").count();
    assert_eq!(grants, 1, "{caps}");
    // The board hears about it
    let mut heard = false;
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::CapabilityChanged { capability } = ev {
            heard |= capability["proposal"]["status"] == "granted" && capability["space_id"] == "spc_default";
        }
    }
    assert!(heard);
    // A retry with the same Idempotency-Key replays the answer
    seed(&e, vec![Seed { class: "ops", ..verified() }; 5]).await;
    set_tier(&e, "ops", "observe").await;
    assert!(!propose(&e, "ops").await.0);
    let pid = proposals(&e, "proposed").await[0]["id"].as_str().unwrap().to_string();
    let req = || {
        Request::builder()
            .method("POST")
            .uri(format!("/api/capabilities/proposals/{pid}/grant"))
            .header("authorization", format!("Bearer {}", e.state.api_token.reveal()))
            .header("idempotency-key", "grant-ops-1")
            .body(Body::empty())
            .unwrap()
    };
    let a = e.app.clone().oneshot(req()).await.unwrap();
    assert_eq!(a.status(), StatusCode::OK);
    let b = e.app.clone().oneshot(req()).await.unwrap();
    assert_eq!(b.status(), StatusCode::OK);
    assert_eq!(b.headers().get("idempotent-replayed").map(|v| v.to_str().unwrap()), Some("true"));
    assert_eq!(live_rows(&e, "ops").await.len(), 1);
}

#[tokio::test]
async fn a_grant_to_act_keeps_the_payment_floor() {
    let e = setup().await;
    person(&e, "PUT", &format!("/api/kwants/{}/approvals", e.bot.id), json!({"approvals": "off"})).await;
    flight_school(&e, true).await;
    set_tier(&e, "general", "sandbox").await;
    seed(&e, vec![Seed { class: "general", ..verified() }; 10]).await;
    let (err, text) = propose(&e, "general").await;
    assert!(!err, "{text}");
    let pid = proposals(&e, "proposed").await[0]["id"].as_str().unwrap().to_string();
    person(&e, "POST", &format!("/api/capabilities/proposals/{pid}/grant"), json!(null)).await;
    assert_eq!(live_rows(&e, "general").await[0]["tier"], "act");
    let (_tid, dir) = claimed_task(&e, "general", true).await;
    let Err(d) = claude_asks(&e, &dir, "Bash", json!({"command": STRIPE_POST})).await else { panic!("a payment asks at a granted act") };
    assert!(d["payment_reason"].as_str().unwrap().contains("stripe.com"), "{d}");
    // And a revoke is still instant after a grant
    person(&e, "POST", &format!("/api/capabilities/{}/revoke", e.bot.id), json!({"task_class": "general", "to_tier": "observe"})).await;
    let Err(d) = claude_asks(&e, &dir, "Write", json!({"file_path": dir.join("a.md").display().to_string(), "content": "x"})).await else {
        panic!("observe cards a write")
    };
    assert!(d["tier_reason"].as_str().is_some(), "{d}");
}
