//! Parallel Universe (`parallel`): the person-only scenario API, validation, the estimate route,
//! and (ignored without Docker) the container harness against a tiny synthetic app.
//!
//! `cargo test --test parallel_test -- --ignored --nocapture` runs the harness tests; they need
//! Docker, the `kwnt-executor:v1` image and `ghcr.io/hatoo/oha:1.16.0`.

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
        harness::{self, BranchSpec, HarnessError, Names},
        AppSpec, Limits, LoadSpec, Tag,
    },
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    Env { app: create_router(state.clone()), state, _tmp: tmp }
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
    ];
    for (method, uri, body) in &routes {
        let (s, v) = call(&e.app, method, uri, None, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
        let (s, v) = call(&e.app, method, uri, Some("not-the-token"), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a wrong token: {v}");
    }
    assert!(e.state.db.pu_scenarios().await.unwrap().is_empty(), "nothing was stored by a refused call");
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
