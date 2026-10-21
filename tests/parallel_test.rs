//! Parallel Universe (`parallel`): the person-only scenario API, validation, the estimate route,
//! and (ignored without Docker) the container harness against a tiny synthetic app.
//!
//! `cargo test --test parallel_test -- --ignored --nocapture` runs the harness tests; they need
//! Docker, the `kwnt-executor:v1` image and oha 1.16.0 (`harness::OHA_IMAGE`, pinned by digest).
//! The run routes are tested with a fake harness, so they need no Docker.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    parallel::{
        harness::{self, BranchOutcome, BranchSpec, HarnessError, Names},
        run::{self, BoxFut, Harness, Run, RunStatus, Runner},
        AppSpec, Figure, Limits, LoadSpec, Tag,
    },
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> Env {
    setup_with(None).await
}

async fn setup_with(harness: Option<Arc<dyn Harness>>) -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    if let Some(h) = harness {
        state.parallel = Arc::new(Runner::with_harness(h));
    }
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

/// Measures nothing: an option called `dead` never gets ready, every other one is fast. A held
/// gate keeps the first branch running until the test releases it.
#[derive(Default)]
struct Fake {
    gate: Option<Arc<tokio::sync::Semaphore>>,
    branches: Mutex<Vec<BranchSpec>>,
    swept: Mutex<Vec<String>>,
    leftovers: usize,
}

impl Harness for Fake {
    fn run_branch(&self, spec: BranchSpec) -> BoxFut<BranchOutcome> {
        self.branches.lock().unwrap().push(spec.clone());
        let gate = self.gate.clone();
        Box::pin(async move {
            if let Some(g) = gate {
                let _ = g.acquire().await.unwrap();
            }
            let at = chrono::Utc::now();
            let result = if spec.branch == "dead" {
                Err(HarnessError::NotReady)
            } else {
                let p95 = if spec.branch == "vps" { 8.0 } else { 3.0 };
                Ok(vec![
                    Figure::verified("p95_latency", p95, "ms", "docker run … oha", "{\"fake\":true}", at),
                    Figure::verified("requests_per_sec", 900.0, "1/s", "docker run … oha", "{\"fake\":true}", at),
                    Figure::verified("ok_share", 1.0, "ratio", "docker run … oha", "{\"fake\":true}", at),
                    Figure::verified("memory_after_load", 21.5, "MB", "docker stats", "21.5MiB / 512MiB", at),
                ])
            };
            BranchOutcome { result, cleanup_errors: Vec::new() }
        })
    }
    fn sweep(&self, run_id: String) -> BoxFut<Result<usize, String>> {
        self.swept.lock().unwrap().push(run_id);
        let n = self.leftovers;
        Box::pin(async move { Ok(n) })
    }
}

/// `call` from another machine (a non-loopback Host), without a token.
async fn call_remote(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").header("host", "kwnt.example.com");
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Env {
    /// Polls a run until it is no longer running.
    async fn finished(&self, run_id: &str) -> Value {
        for _ in 0..500 {
            let (s, v) = self.person("GET", &format!("/api/parallel/runs/{run_id}"), None).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            if v["run"]["status"] != "running" {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("run {run_id} never finished");
    }
}

async fn call(app: &Router, method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Env {
    async fn person(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        call(&self.app, method, uri, Some(self.state.api_token.reveal()), body).await
    }
    async fn turn_token(&self, folder: &Path) -> String {
        let bot = self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
        let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
        mcp_runtime::prepare_turn_with_self(&self.state, &bot, None, Some(folder.to_path_buf()), Some(scope)).await.unwrap().1
    }
}

fn scenario() -> Value {
    json!({
        "title": "  Move the shop off managed hosting?  ",
        "question": "Should I move my app from managed hosting to my own VPS?",
        "requirements": {"notes": "Postgres stays where it is", "needs_persistent_disk": false, "regions": ["eu"]},
        "traffic": {"monthly_requests": 3000000, "peak_rps": 40, "avg_response_kb": 25, "avg_duration_ms": 80},
        "load": {"requests": 500, "concurrency": 8, "path": "/"},
        "criteria": [{"key": "cost", "weight": 5}, {"key": "p95_latency", "weight": 3}, {"key": "rollback", "weight": 2}],
        "options": [
            {"key": "vps", "label": "Hetzner VPS", "plan": "hetzner-cpx12",
             "app": {"image": "kwnt-executor:v1", "port": 8080}, "limits": {"cpus": 1.0, "memory_mb": 512, "pids": 128, "timeout_secs": 120}},
            {"key": "fly", "label": "Fly.io", "plan": "fly-shared-1x-512", "app": {"context": "/tmp/app", "port": 8080}},
            {"key": "lambda", "label": "Lambda", "plan": "aws-lambda-x86-use1", "app": {"image": "kwnt-executor:v1", "port": 8080}}
        ]
    })
}

#[tokio::test]
async fn every_route_is_the_persons_only() {
    let e = setup().await;
    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;
    let routes: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/api/parallel/scenarios", None),
        ("POST", "/api/parallel/scenarios", Some(scenario())),
        ("GET", "/api/parallel/scenarios/x", None),
        ("PUT", "/api/parallel/scenarios/x", Some(scenario())),
        ("DELETE", "/api/parallel/scenarios/x", None),
        ("GET", "/api/parallel/prices", None),
        ("POST", "/api/parallel/estimate", Some(json!({"plan": "do-basic-1gb", "traffic": {"monthly_requests": 1}}))),
        ("POST", "/api/parallel/scenarios/x/run", None),
        ("GET", "/api/parallel/scenarios/x/runs", None),
        ("GET", "/api/parallel/runs", None),
        ("GET", "/api/parallel/runs/x", None),
    ];
    for (method, uri, body) in &routes {
        let (s, v) = call_remote(&e.app, method, uri, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} from another machine without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
        let (s, v) = call(&e.app, method, uri, Some("not-the-token"), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a wrong token: {v}");
    }
    assert!(e.state.db.pu_scenarios().await.unwrap().is_empty(), "nothing was stored by a refused call");
    assert!(e.state.db.pu_runs(None, 10).await.unwrap().is_empty(), "no run was started by a refused call");
}

#[tokio::test]
async fn the_person_keeps_scenarios() {
    let e = setup().await;
    let (s, created) = e.person("POST", "/api/parallel/scenarios", Some(scenario())).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["title"], "Move the shop off managed hosting?");
    assert_eq!(created["options"][1]["limits"], json!({"cpus": 1.0, "memory_mb": 512, "pids": 128, "timeout_secs": 300}), "defaults filled in");
    assert_eq!(created["options"][0]["app"]["health_path"], "/");

    let (s, list) = e.person("GET", "/api/parallel/scenarios", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["scenarios"].as_array().unwrap().len(), 1);
    let (_, got) = e.person("GET", &format!("/api/parallel/scenarios/{id}"), None).await;
    assert_eq!(got, created);

    let mut changed = scenario();
    changed["title"] = json!("VPS or not");
    changed["options"].as_array_mut().unwrap().truncate(2);
    changed["id"] = json!("someone-elses-id");
    let (s, updated) = e.person("PUT", &format!("/api/parallel/scenarios/{id}"), Some(changed)).await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    assert_eq!((updated["id"].as_str(), updated["title"].as_str()), (Some(id.as_str()), Some("VPS or not")));
    assert_eq!(updated["created_at"], created["created_at"]);
    assert_eq!(updated["options"].as_array().unwrap().len(), 2);

    let (s, _) = e.person("DELETE", &format!("/api/parallel/scenarios/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = e.person("GET", &format!("/api/parallel/scenarios/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("DELETE", &format!("/api/parallel/scenarios/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bad_scenarios_are_refused_and_not_stored() {
    let e = setup().await;
    type Change = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Change)> = vec![
        ("four options", Box::new(|v| { let o = v["options"][0].clone(); let mut o4 = o.clone(); o4["key"] = json!("four"); v["options"].as_array_mut().unwrap().push(o4); })),
        ("no options", Box::new(|v| v["options"] = json!([]))),
        ("duplicate key", Box::new(|v| v["options"][1]["key"] = json!("vps"))),
        ("key not a slug", Box::new(|v| v["options"][0]["key"] = json!("VPS; rm"))),
        ("unknown plan", Box::new(|v| v["options"][0]["plan"] = json!("aws-ec2-free"))),
        ("image that is a flag", Box::new(|v| v["options"][0]["app"]["image"] = json!("--privileged"))),
        ("image and context", Box::new(|v| v["options"][0]["app"]["context"] = json!("/tmp/app"))),
        ("relative context", Box::new(|v| v["options"][1]["app"]["context"] = json!("app"))),
        ("context with ..", Box::new(|v| v["options"][1]["app"]["context"] = json!("/tmp/../etc"))),
        ("dockerfile outside", Box::new(|v| v["options"][1]["app"]["dockerfile"] = json!("../Dockerfile"))),
        ("port 0", Box::new(|v| v["options"][0]["app"]["port"] = json!(0))),
        ("too much cpu", Box::new(|v| v["options"][0]["limits"]["cpus"] = json!(8))),
        ("too much memory", Box::new(|v| v["options"][0]["limits"]["memory_mb"] = json!(4096))),
        ("too much memory together", Box::new(|v| for o in v["options"].as_array_mut().unwrap() { o["limits"] = json!({"cpus": 1, "memory_mb": 2048, "pids": 128, "timeout_secs": 60}); })),
        ("timeout too long", Box::new(|v| v["options"][0]["limits"]["timeout_secs"] = json!(3600))),
        ("load too big", Box::new(|v| v["load"]["concurrency"] = json!(1000))),
        ("load path", Box::new(|v| v["load"]["path"] = json!("http://prod.example.com/"))),
        ("criterion twice", Box::new(|v| v["criteria"] = json!([{"key": "cost", "weight": 1}, {"key": "cost", "weight": 2}]))),
        ("unknown criterion", Box::new(|v| v["criteria"] = json!([{"key": "vibes", "weight": 1}]))),
        ("empty title", Box::new(|v| v["title"] = json!("   "))),
        ("negative traffic", Box::new(|v| v["traffic"]["peak_rps"] = json!(-1))),
    ];
    for (name, change) in cases {
        let mut v = scenario();
        change(&mut v);
        let (s, body) = e.person("POST", "/api/parallel/scenarios", Some(v)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert!(body["error"].is_string(), "{name}: {body}");
    }
    assert!(e.state.db.pu_scenarios().await.unwrap().is_empty());
}

#[tokio::test]
async fn prices_and_estimates_name_their_source() {
    let e = setup().await;
    let (s, v) = e.person("GET", "/api/parallel/prices", None).await;
    assert_eq!(s, StatusCode::OK);
    let prices = v["prices"].as_array().unwrap();
    assert!(prices.iter().all(|p| p["source_url"].as_str().unwrap().starts_with("https://") && p["as_of"].is_string()));

    let (s, v) = e.person("POST", "/api/parallel/estimate", Some(json!({"plan": "do-basic-1gb", "traffic": {"monthly_requests": 1000000, "avg_response_kb": 20}}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let f = &v["figures"][0];
    assert_eq!((f["tag"].as_str(), f["value"].as_f64(), f["unit"].as_str()), (Some("estimate"), Some(6.0), Some("USD")));
    assert_eq!(f["source_url"], "https://www.digitalocean.com/pricing/droplets");
    assert!(!f["assumptions"].as_array().unwrap().is_empty());

    let (_, v) = e.person("POST", "/api/parallel/estimate", Some(json!({"plan": null, "traffic": {"monthly_requests": 1}}))).await;
    assert_eq!(v["figures"][0]["tag"], "speculation");
    let (s, _) = e.person("POST", "/api/parallel/estimate", Some(json!({"plan": "nope", "traffic": {"monthly_requests": 1}}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

fn run_scenario() -> Value {
    let mut v = scenario();
    v["options"] = json!([
        {"key": "vps", "label": "DigitalOcean droplet", "plan": "do-basic-1gb", "app": {"image": "kwnt-executor:v1", "port": 8080},
         "rollback_notes": "keep the last two releases"},
        {"key": "fly", "label": "Fly.io", "plan": "fly-shared-1x-512", "app": {"context": "/tmp/app", "port": 8080}},
        {"key": "dead", "label": "Never answers", "app": {"image": "kwnt-executor:v1", "port": 8080}}
    ]);
    v
}

#[tokio::test]
async fn a_run_tries_every_option_and_reports_tagged_numbers() {
    let fake = Arc::new(Fake::default());
    let e = setup_with(Some(fake.clone())).await;
    let (_, created) = e.person("POST", "/api/parallel/scenarios", Some(run_scenario())).await;
    let id = created["id"].as_str().unwrap().to_string();

    let (s, v) = e.person("POST", "/api/parallel/scenarios/nope/run", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, started) = e.person("POST", &format!("/api/parallel/scenarios/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["run"]["status"], "running");
    let run_id = started["run"]["id"].as_str().unwrap().to_string();

    let done = e.finished(&run_id).await;
    let (run, report) = (&done["run"], &done["report"]);
    assert_eq!(run["status"], "finished");
    assert!(run["finished_at"].is_string());
    let statuses: Vec<&str> = run["branches"].as_array().unwrap().iter().map(|b| b["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["ok", "ok", "failed"]);
    assert_eq!(run["branches"][2]["error_code"], "not_ready");
    assert_eq!(run["branches"][1]["build_network_used"], false, "builds stay offline by default");

    // the harness got each option, in order, with the run's own id and the scenario's load
    let specs = fake.branches.lock().unwrap().clone();
    assert_eq!(specs.iter().map(|b| b.branch.as_str()).collect::<Vec<_>>(), ["vps", "fly", "dead"]);
    assert!(specs.iter().all(|b| b.run_id == run_id && b.load.requests == 500 && !b.build_network));
    assert_eq!(fake.swept.lock().unwrap().as_slice(), std::slice::from_ref(&run_id), "the run's label is swept at the end");

    // the report: measured = verified, cost = estimate or speculation, notes = speculation
    let opts = report["options"].as_array().unwrap();
    for o in opts {
        assert!(o["measured"].as_array().unwrap().iter().all(|f| f["tag"] == "verified" && f["command"].is_string() && f["raw_output"].is_string()));
        assert!(o["notes"].as_array().unwrap().iter().all(|n| n["tag"] == "speculation"));
    }
    assert_eq!(opts[0]["cost"]["tag"], "estimate");
    assert_eq!(opts[0]["cost"]["source_url"], "https://www.digitalocean.com/pricing/droplets");
    assert!(opts[0]["cost"]["as_of"].is_string() && opts[0]["cost"]["stale"] == false);
    assert_eq!(opts[2]["cost"]["tag"], "speculation");
    assert!(opts[2]["measured"].as_array().unwrap().is_empty());
    assert_eq!(opts[0]["notes"][1]["text"], "keep the last two releases");
    let leaders = report["leaders"].as_array().unwrap();
    assert!(leaders.iter().any(|l| l["criterion"] == "p95_latency" && l["option"] == "fly" && l["basis"] == "verified"), "{leaders:?}");
    assert!(leaders.iter().any(|l| l["criterion"] == "cost" && l["basis"] == "estimate"), "{leaders:?}");
    assert!(report["caveats"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("not on the provider's hardware")));

    // a run keeps the scenario as it was: editing it later changes nothing
    let mut changed = run_scenario();
    changed["title"] = json!("Something else");
    e.person("PUT", &format!("/api/parallel/scenarios/{id}"), Some(changed)).await;
    let (_, again) = e.person("GET", &format!("/api/parallel/runs/{run_id}"), None).await;
    assert_eq!(again["report"]["title"], "Move the shop off managed hosting?");

    let (_, list) = e.person("GET", "/api/parallel/runs", None).await;
    assert_eq!((list["runs"].as_array().unwrap().len(), &list["running"]), (1, &json!(false)));
    let (_, mine) = e.person("GET", &format!("/api/parallel/scenarios/{id}/runs"), None).await;
    assert_eq!(mine["runs"][0]["id"], run_id.as_str());
    let (s, _) = e.person("GET", "/api/parallel/runs/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_one_run_goes_at_a_time() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let fake = Arc::new(Fake { gate: Some(gate.clone()), ..Default::default() });
    let e = setup_with(Some(fake.clone())).await;
    let (_, a) = e.person("POST", "/api/parallel/scenarios", Some(run_scenario())).await;
    let (_, b) = e.person("POST", "/api/parallel/scenarios", Some(scenario())).await;
    let (s, first) = e.person("POST", &format!("/api/parallel/scenarios/{}/run", a["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // another scenario's run is refused too: the limit is server-wide
    let (s, v) = e.person("POST", &format!("/api/parallel/scenarios/{}/run", b["id"].as_str().unwrap()), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("busy")), "{v}");
    let (_, list) = e.person("GET", "/api/parallel/runs", None).await;
    assert_eq!((list["runs"].as_array().unwrap().len(), &list["running"]), (1, &json!(true)), "the refused start stored nothing");
    let mut poll = Value::Null;
    for _ in 0..500 {
        poll = e.person("GET", &format!("/api/parallel/runs/{}", first["run"]["id"].as_str().unwrap()), None).await.1;
        if poll["run"]["branches"][0]["status"] == "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(poll["run"]["status"], "running");
    assert_eq!(poll["run"]["branches"][0]["status"], "running");
    assert_eq!(poll["run"]["branches"][1]["status"], "pending");

    gate.add_permits(10);
    e.finished(first["run"]["id"].as_str().unwrap()).await;
    let (s, v) = e.person("POST", &format!("/api/parallel/scenarios/{}/run", b["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "free again once it finished: {v}");
    e.finished(v["run"]["id"].as_str().unwrap()).await;
}

#[tokio::test]
async fn the_build_network_is_off_unless_asked_and_said_when_used() {
    let fake = Arc::new(Fake::default());
    let e = setup_with(Some(fake.clone())).await;
    let (_, plain) = e.person("POST", "/api/parallel/scenarios", Some(run_scenario())).await;
    assert_eq!(plain["build_network"], false);
    let mut v = run_scenario();
    v["build_network"] = json!(true);
    let (_, net) = e.person("POST", "/api/parallel/scenarios", Some(v)).await;
    assert_eq!(net["build_network"], true);
    let (_, got) = e.person("GET", &format!("/api/parallel/scenarios/{}", net["id"].as_str().unwrap()), None).await;
    assert_eq!(got["build_network"], true, "stored");

    let (_, started) = e.person("POST", &format!("/api/parallel/scenarios/{}/run", net["id"].as_str().unwrap()), None).await;
    let done = e.finished(started["run"]["id"].as_str().unwrap()).await;
    let used: Vec<bool> = done["run"]["branches"].as_array().unwrap().iter().map(|b| b["build_network_used"].as_bool().unwrap()).collect();
    assert_eq!(used, [false, true, false], "only the option that builds (fly) had it");
    assert!(fake.branches.lock().unwrap().iter().all(|b| b.build_network));
    assert_eq!(done["report"]["build_network_used"], true);
    assert!(done["report"]["caveats"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("network access for: fly")));
}

#[tokio::test]
async fn a_run_a_stopped_server_left_behind_is_marked_and_swept_once() {
    let e = setup().await;
    let (_, created) = e.person("POST", "/api/parallel/scenarios", Some(run_scenario())).await;
    let scenario = e.state.db.pu_scenario(created["id"].as_str().unwrap()).await.unwrap().unwrap();
    let mut stuck = Run::new(&scenario);
    stuck.branches[0].status = run::BranchStatus::Ok;
    stuck.branches[1].status = run::BranchStatus::Running;
    e.state.db.pu_put_run(&stuck).await.unwrap();
    let mut clean = Run::new(&scenario);
    clean.status = RunStatus::Finished;
    e.state.db.pu_put_run(&clean).await.unwrap();

    let fake = Fake { leftovers: 2, ..Default::default() };
    assert_eq!(run::startup(&e.state.db, &fake).await.unwrap(), 2);
    assert_eq!(fake.swept.lock().unwrap().as_slice(), std::slice::from_ref(&stuck.id), "a finished, clean run is left alone");
    let after = e.state.db.pu_run(&stuck.id).await.unwrap().unwrap();
    assert_eq!(after.status, RunStatus::Interrupted);
    assert_eq!(after.branches.iter().map(|b| b.status).collect::<Vec<_>>(), [run::BranchStatus::Ok, run::BranchStatus::Failed, run::BranchStatus::Failed]);
    assert_eq!(after.branches[1].error_code.as_deref(), Some("interrupted"));
    assert!(after.swept_at.is_some() && after.cleanup_errors.iter().any(|c| c.contains("removed 2")));

    let again = Fake::default();
    assert_eq!(run::startup(&e.state.db, &again).await.unwrap(), 0);
    assert!(again.swept.lock().unwrap().is_empty(), "swept runs are not swept again");
}

#[tokio::test]
async fn a_slice_1_database_gains_build_network() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let path = data.join("kwnt.db");
    assert!(path.exists(), "{path:?}");
    let url = format!("sqlite://{}", path.display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE pu_runs").execute(&pool).await.unwrap();
    sqlx::query("ALTER TABLE pu_scenarios DROP COLUMN build_network").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO pu_scenarios (id, title, question, requirements, traffic, load, criteria, options, created_at, updated_at)
                 VALUES ('old', 'Old', '', '{}', '{\"monthly_requests\":1}', '{\"requests\":1,\"concurrency\":1,\"path\":\"/\"}', '[]', '[]', '2026-10-20T00:00:00Z', '2026-10-20T00:00:00Z')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    drop(db);
    let db = Database::init(&data).await.unwrap();
    let old = db.pu_scenario("old").await.unwrap().unwrap();
    assert!(!old.build_network);
    assert!(db.pu_runs(None, 10).await.unwrap().is_empty());
}

// ---- the harness against real Docker (ignored: needs Docker and two local images) ----

fn docker(args: &[&str]) -> String {
    let o = std::process::Command::new("docker").args(args).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Every object a run could leave behind.
fn leftovers(run_id: &str) -> String {
    let filter = format!("label=kwnt.pu.run={run_id}");
    [docker(&["ps", "-aq", "--filter", &filter]), docker(&["network", "ls", "-q", "--filter", &filter]), docker(&["images", "-q", "--filter", &filter])]
        .join(" ")
        .trim()
        .to_string()
}

/// A tiny app: Python's file server, built offline from the local executor image.
fn synthetic_app(dir: &Path, cmd: &str) -> AppSpec {
    std::fs::write(dir.join("Dockerfile"), format!("FROM kwnt-executor:v1\nWORKDIR /srv\nRUN echo hello > index.html\nCMD {cmd}\n")).unwrap();
    AppSpec { image: None, context: Some(dir.to_string_lossy().into()), dockerfile: None, port: 8080, health_path: "/".into() }
}

#[tokio::test]
#[ignore]
async fn harness_measures_a_synthetic_app_and_leaves_nothing_behind() {
    let ctx = tempdir().unwrap();
    let run_id = format!("pu{}", uuid::Uuid::new_v4().simple());
    let spec = BranchSpec {
        run_id: run_id.clone(),
        branch: "vps".into(),
        app: synthetic_app(ctx.path(), r#"["python3", "-m", "http.server", "8080"]"#),
        limits: Limits { cpus: 0.5, memory_mb: 128, pids: 64, timeout_secs: 120 },
        load: LoadSpec { requests: 300, concurrency: 5, path: "/".into() },
        build_network: false,
    };
    let out = harness::run_branch(&spec).await;
    let figures = out.result.expect("the branch ran");
    assert!(out.cleanup_errors.is_empty(), "{:?}", out.cleanup_errors);
    for f in &figures {
        println!("{} = {} {}", f.metric, f.value, f.unit);
        let Tag::Verified { command, raw_output, .. } = &f.tag else { panic!("{f:?}") };
        assert!(command.starts_with("docker "), "{command}");
        assert!(!raw_output.is_empty());
    }
    let get = |m: &str| figures.iter().find(|f| f.metric == m).map(|f| f.value);
    assert!(get("p95_latency").unwrap() > 0.0);
    assert_eq!(get("ok_share"), Some(1.0));
    assert!(get("memory_after_load").unwrap() > 0.0 && get("memory_after_load").unwrap() <= 128.0);
    assert_eq!(leftovers(&run_id), "", "containers, network and image are gone");
}

#[tokio::test]
#[ignore]
async fn an_app_that_never_answers_is_cleaned_up_too() {
    let ctx = tempdir().unwrap();
    let run_id = format!("pu{}", uuid::Uuid::new_v4().simple());
    let spec = BranchSpec {
        run_id: run_id.clone(),
        branch: "dead".into(),
        app: synthetic_app(ctx.path(), r#"["sleep", "infinity"]"#),
        limits: Limits { cpus: 0.25, memory_mb: 64, pids: 16, timeout_secs: 30 },
        load: LoadSpec::default(),
        build_network: false,
    };
    let out = harness::run_branch(&spec).await;
    assert!(matches!(out.result, Err(HarnessError::NotReady) | Err(HarnessError::Timeout)), "{:?}", out.result);
    assert!(out.cleanup_errors.is_empty(), "{:?}", out.cleanup_errors);
    assert_eq!(leftovers(&run_id), "");
}

#[tokio::test]
#[ignore]
async fn the_internal_network_reaches_neither_the_internet_nor_the_host() {
    let run_id = format!("pu{}", uuid::Uuid::new_v4().simple());
    let n = Names::new(&run_id, "probe");
    let args = harness::network_args(&n);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    docker(&args);
    let gateway = docker(&["network", "inspect", "bridge", "-f", "{{(index .IPAM.Config 0).Gateway}}"]);
    let try_url = |url: &str| {
        let o = std::process::Command::new("docker")
            .args(["run", "--rm", "--label", &n.label, "--network", &n.network, "kwnt-executor:v1", "curl", "-s", "-m", "5", "-o", "/dev/null", "-w", "%{http_code}", url])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    // the branch network's own gateway is the host too
    let own_gateway = docker(&["network", "inspect", &n.network, "-f", "{{(index .IPAM.Config 0).Gateway}}"]);
    assert!(!own_gateway.is_empty());
    let mut urls = vec!["https://example.com/".to_string(), "http://host.docker.internal:8799/".into()];
    for gw in [&gateway, &own_gateway] {
        urls.extend([format!("http://{gw}:8799/"), format!("http://{gw}:8800/")]);
    }
    for url in &urls {
        assert_eq!(try_url(url), "000", "{url} must be unreachable from the branch network");
    }
    // and there is no default route out at all
    let routes = std::process::Command::new("docker")
        .args(["run", "--rm", "--label", &n.label, "--network", &n.network, "kwnt-executor:v1", "cat", "/proc/net/route"])
        .output()
        .unwrap();
    let routes = String::from_utf8_lossy(&routes.stdout);
    assert!(!routes.lines().skip(1).any(|l| l.split_whitespace().nth(1) == Some("00000000")), "{routes}");
    assert!(harness::sweep(Some(&run_id)).await.unwrap() >= 1);
    assert_eq!(leftovers(&run_id), "");
}

#[tokio::test]
#[ignore]
async fn the_build_reaches_the_network_only_when_asked() {
    let ctx = tempdir().unwrap();
    // the build step downloads nothing, it only opens a connection to a public resolver
    std::fs::write(
        ctx.path().join("Dockerfile"),
        "FROM kwnt-executor:v1\nWORKDIR /srv\nRUN python3 -c \"import socket; socket.create_connection(('1.1.1.1', 53), 5)\" && echo ok > index.html\nCMD [\"python3\", \"-m\", \"http.server\", \"8080\"]\n",
    )
    .unwrap();
    let app = AppSpec { image: None, context: Some(ctx.path().to_string_lossy().into()), dockerfile: None, port: 8080, health_path: "/".into() };
    let limits = Limits { cpus: 0.5, memory_mb: 128, pids: 64, timeout_secs: 120 };
    let load = LoadSpec { requests: 100, concurrency: 2, path: "/".into() };
    for (build_network, expect_ok) in [(false, false), (true, true)] {
        let run_id = format!("pu{}", uuid::Uuid::new_v4().simple());
        let spec = BranchSpec { run_id: run_id.clone(), branch: "net".into(), app: app.clone(), limits, load: load.clone(), build_network };
        let out = harness::run_branch(&spec).await;
        match (&out.result, expect_ok) {
            (Ok(_), true) => {}
            (Err(HarnessError::Build(_)), false) => {}
            (r, _) => panic!("build_network={build_network}: {r:?}"),
        }
        assert!(out.cleanup_errors.is_empty(), "{:?}", out.cleanup_errors);
        assert_eq!(leftovers(&run_id), "", "build_network={build_network}");
    }
}

#[tokio::test]
#[ignore]
async fn a_real_run_through_the_routes_leaves_nothing_behind() {
    let e = setup_with(Some(Arc::new(run::DockerHarness))).await;
    let a = tempdir().unwrap();
    let b = tempdir().unwrap();
    synthetic_app(a.path(), r#"["python3", "-m", "http.server", "8080"]"#);
    synthetic_app(b.path(), r#"["python3", "-m", "http.server", "8080", "--bind", "0.0.0.0"]"#);
    let mut v = scenario();
    v["load"] = json!({"requests": 200, "concurrency": 4, "path": "/"});
    v["options"] = json!([
        {"key": "vps", "label": "DO droplet", "plan": "do-basic-1gb", "app": {"context": a.path(), "port": 8080},
         "limits": {"cpus": 0.5, "memory_mb": 128, "pids": 64, "timeout_secs": 120}},
        {"key": "fly", "label": "Fly.io", "plan": "fly-shared-1x-512", "app": {"context": b.path(), "port": 8080},
         "limits": {"cpus": 0.25, "memory_mb": 96, "pids": 64, "timeout_secs": 120}}
    ]);
    let (_, created) = e.person("POST", "/api/parallel/scenarios", Some(v)).await;
    let (s, started) = e.person("POST", &format!("/api/parallel/scenarios/{}/run", created["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{started}");
    let run_id = started["run"]["id"].as_str().unwrap().to_string();
    let mut done = Value::Null;
    for _ in 0..600 {
        let (_, v) = e.person("GET", &format!("/api/parallel/runs/{run_id}"), None).await;
        if v["run"]["status"] != "running" {
            done = v;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!("{}", serde_json::to_string_pretty(&done["report"]).unwrap());
    assert_eq!(done["run"]["status"], "finished", "{done}");
    for b in done["run"]["branches"].as_array().unwrap() {
        assert_eq!(b["status"], "ok", "{b}");
        assert!(b["cleanup_errors"].as_array().unwrap().is_empty(), "{b}");
    }
    assert!(done["run"]["cleanup_errors"].as_array().unwrap().is_empty());
    assert_eq!(leftovers(&run_id), "");
}
