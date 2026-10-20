//! Saved passwords in the person's Bitwarden vault (`sandbox::bitwarden`).
//!
//! The vault routes are the person's only: no token 401, a bot's turn token 403. The rest runs
//! against a real Bitwarden-compatible server when `KWNT_TEST_VAULTWARDEN` names one, e.g. a
//! throwaway Vaultwarden with signups open:
//!
//! ```sh
//! docker run -d --rm --name kwnt-bwtest -p 127.0.0.1:18990:80 -e SIGNUPS_ALLOWED=true \
//!   -e I_REALLY_WANT_VOLATILE_STORAGE=true vaultwarden/server
//! KWNT_TEST_VAULTWARDEN=http://127.0.0.1:18990 cargo test --test bitwarden_test
//! ```
//!
//! Without it those tests pass without doing anything (and say so).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewTeam, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{bitwarden, browser::BrowserController, credentials::CredentialStore, PendingCapture},
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    data: PathBuf,
    _tmp: TempDir,
}

fn config_for(data: &Path) -> Config {
    Config { data_dir: data.to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() }
}

async fn setup_at(tmp: TempDir) -> Env {
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = config_for(&data);
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    Env { app: create_router(state.clone()), state, data, _tmp: tmp }
}

async fn setup() -> Env {
    setup_at(tempdir().unwrap()).await
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
    async fn team(&self, name: &str) -> String {
        let bot = self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
        let team = NewTeam {
            name: name.into(),
            description: None,
            icon: None,
            lead_kwant_id: bot.id.clone(),
            member_kwants: vec![bot.id],
            workspace_path: None,
            repo_url: None,
        };
        self.state.db.create_team(&SpaceId::default_space(), &team).await.unwrap().id
    }
    async fn turn_token(&self, folder: &Path) -> String {
        let bot = self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
        let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
        mcp_runtime::prepare_turn_with_self(&self.state, &bot, None, Some(folder.to_path_buf()), Some(scope))
            .await
            .unwrap()
            .1
    }
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn no_file_contains(dir: &Path, needle: &str) {
    for f in files_under(dir) {
        let bytes = std::fs::read(&f).unwrap_or_default();
        assert!(!String::from_utf8_lossy(&bytes).contains(needle), "{needle} in clear in {}", f.display());
    }
}

#[tokio::test]
async fn vault_routes_are_the_persons_only() {
    let e = setup().await;
    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;
    let routes: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/api/passwords/store", None),
        ("POST", "/api/passwords/bitwarden/unlock", Some(json!({"server_url": "http://127.0.0.1:9", "email": "a@b.c", "master_password": "DUMMY"}))),
        ("POST", "/api/passwords/bitwarden/lock", None),
        ("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": "DUMMY"}))),
        ("POST", "/api/passwords/migrate", None),
    ];
    for (method, uri, body) in &routes {
        let (s, v) = call(&e.app, method, uri, None, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
    }
    let (s, v) = e.person("GET", "/api/passwords/store", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["store"], "local");
    assert_eq!(v["locked"], false);
    assert_eq!(v["bitwarden"], Value::Null);
    // Not set up: nothing to lock, forget or move into
    let (s, _) = e.person("POST", "/api/passwords/migrate", None).await;
    assert_eq!(s, StatusCode::LOCKED);
    // A plain-http server elsewhere is refused before anything is sent
    let (s, v) = e
        .person("POST", "/api/passwords/bitwarden/unlock", Some(json!({"server_url": "http://pw.example.org", "email": "a@b.c", "master_password": "DUMMY"})))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(!e.data.join("kwnt-vault").join(bitwarden::CONFIG_FILE).exists());
}

fn server() -> Option<String> {
    let s = std::env::var("KWNT_TEST_VAULTWARDEN").ok().filter(|s| !s.is_empty());
    if s.is_none() {
        eprintln!("KWNT_TEST_VAULTWARDEN is not set: skipping the live Bitwarden test");
    }
    s
}

const MASTER: &str = "DUMMY-master-pw-kwnt-test";

async fn new_account(server: &str) -> String {
    let email = format!("kwnt-{}@kwnt.test", uuid::Uuid::new_v4().simple());
    // Vaultwarden's minimum; real accounts use 600000
    bitwarden::register_test_account(server, &email, MASTER, 100_000).await.expect("register a throwaway account");
    email
}

/// The person unlocks, passwords are saved, filled on the exact origin only, survive in the
/// vault, and after a restart nothing is filled until the person unlocks again. Nothing that
/// unlocks the vault, and no saved password, is ever written under kwnt's data folder.
#[tokio::test]
async fn bitwarden_end_to_end() {
    let Some(server) = server() else { return };
    let email = new_account(&server).await;
    let tmp = tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();
    let e = setup_at(tmp).await;
    let team = e.team("bwteam").await;

    // A local password from before Bitwarden
    let (s, _) = e
        .person("POST", &format!("/api/sandbox/{team}/passwords"), Some(json!({"origin": "https://old.example.com", "username": "olduser", "password": "DUMMY-LOCAL-PW"})))
        .await;
    assert_eq!(s, StatusCode::OK);
    // and one a deleted bot left behind (audit F-11): counted, and moved too
    e.state.credentials.local().save("0b5e1e7e-dead-4bad-8bad-000000000011", "https://orphan.example.com", "ou", "DUMMY-ORPHAN-PW").await.unwrap();

    // Wrong master password
    let unlock = |pw: &str| json!({"server_url": server, "email": email, "master_password": pw});
    let (s, v) = e.person("POST", "/api/passwords/bitwarden/unlock", Some(unlock("DUMMY-wrong"))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["wrong_password"], true);
    assert!(!e.data.join("kwnt-vault").join(bitwarden::CONFIG_FILE).exists(), "nothing set up on a failed unlock");

    let (s, v) = e.person("POST", "/api/passwords/bitwarden/unlock", Some(unlock(MASTER))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["store"], "bitwarden");
    assert_eq!(v["locked"], false);
    assert_eq!(v["bitwarden"]["email"], email);
    assert_eq!(v["bitwarden"]["unlocked"], true);
    assert_eq!(v["bitwarden"]["login"], "password");
    assert_eq!(v["local_count"], 2);

    // Another account can't replace it without forgetting this one
    let (s, _) = e
        .person("POST", "/api/passwords/bitwarden/unlock", Some(json!({"server_url": server, "email": "other@kwnt.test", "master_password": MASTER})))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Move the local one over
    let (s, v) = e.person("POST", "/api/passwords/migrate", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"moved": 2, "failed": 0}));
    assert_eq!(e.state.credentials.local_count().await, 0);
    let left: Vec<_> = std::fs::read_dir(e.data.join("kwnt-vault"))
        .unwrap()
        .flatten()
        .filter(|f| f.file_name().to_string_lossy().ends_with(".bin"))
        .collect();
    assert!(left.is_empty(), "Move to Bitwarden leaves no built-in file: {left:?}");

    // Save one through the API and one the browser captured
    let uri = format!("/api/sandbox/{team}/passwords");
    let (s, v) = e.person("POST", &uri, Some(json!({"origin": "https://login.example.com/x", "username": "alice", "password": "DUMMY-BW-PW-1"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let cid = v["credential"]["id"].as_str().unwrap().to_string();
    let ctrl = BrowserController::new(9);
    e.state.browser_controllers.lock().await.insert(team.clone(), ctrl.clone());
    ctrl.hold_for_confirmation(PendingCapture {
        origin: "https://shop.example.com".into(),
        username: "bob".into(),
        password: "DUMMY-BW-PW-2".into(),
        captured_at: "2026-10-20T00:00:00Z".into(),
    })
    .await;
    let (s, v) = e.person("POST", &format!("{uri}/pending"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (_, v) = e.person("GET", &uri, None).await;
    let origins: Vec<&str> = v["passwords"].as_array().unwrap().iter().map(|p| p["origin"].as_str().unwrap()).collect();
    assert_eq!(origins.len(), 3, "{v}");
    for o in ["https://old.example.com", "https://login.example.com", "https://shop.example.com"] {
        assert!(origins.contains(&o), "{o} in {v}");
    }
    assert!(!v.to_string().contains("DUMMY"), "metadata only: {v}");

    // Exact origin only
    let vault = &e.state.credentials;
    assert_eq!(vault.for_origin(&team, "https://login.example.com/other").await.unwrap().unwrap().password, "DUMMY-BW-PW-1");
    assert!(vault.for_origin(&team, "https://evil.login.example.com").await.unwrap().is_none());
    assert!(vault.for_origin(&team, "http://login.example.com").await.unwrap().is_none());
    assert!(vault.for_origin("another-owner", "https://login.example.com").await.unwrap().is_none());

    // Reveal needs the master password, not just the API token
    let reveal = format!("{uri}/{cid}/reveal");
    let (s, v) = e.person("POST", &reveal, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert!(!v.to_string().contains("DUMMY-BW"));
    let (s, v) = e.person("POST", &reveal, Some(json!({"master_password": MASTER}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["password"], "DUMMY-BW-PW-1");

    // An update keeps one item
    let (s, _) = e.person("POST", &uri, Some(json!({"origin": "https://login.example.com", "username": "alice", "password": "DUMMY-BW-PW-1b"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = e.person("GET", &uri, None).await;
    assert_eq!(v["passwords"].as_array().unwrap().len(), 3);

    // Nothing secret on disk: no password, master password, or token
    no_file_contains(&tmp_path, "DUMMY-BW-PW");
    no_file_contains(&tmp_path, "DUMMY-LOCAL-PW");
    no_file_contains(&tmp_path, MASTER);
    let cfg = std::fs::read_to_string(e.data.join("kwnt-vault").join(bitwarden::CONFIG_FILE)).unwrap();
    let cfg: Value = serde_json::from_str(&cfg).unwrap();
    let mut keys: Vec<&str> = cfg.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["client_id", "device_id", "email", "server_url"], "{cfg}");

    // A restart: the vault is locked, nothing is filled or listed
    let restarted = kwnt::sandbox::vault::PasswordVault::open(&e.data);
    assert!(restarted.is_locked().await);
    assert!(restarted.for_origin(&team, "https://login.example.com").await.unwrap().is_none());
    assert!(restarted.list(&team).await.is_err());

    // Lock through the API: same
    let (s, v) = e.person("POST", "/api/passwords/bitwarden/lock", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["locked"], true);
    assert_eq!(v["bitwarden"]["unlocked"], false);
    assert!(vault.for_origin(&team, "https://login.example.com").await.unwrap().is_none());
    let (s, v) = e.person("GET", &uri, None).await;
    assert_eq!(s, StatusCode::LOCKED, "{v}");
    assert_eq!(v["locked"], true);
    // A capture made while locked waits instead of being lost
    ctrl.hold_for_confirmation(PendingCapture {
        origin: "https://later.example.com".into(),
        username: "carol".into(),
        password: "DUMMY-BW-PW-3".into(),
        captured_at: "2026-10-20T00:00:00Z".into(),
    })
    .await;
    let (s, _) = e.person("POST", &format!("{uri}/pending"), None).await;
    assert_eq!(s, StatusCode::LOCKED);
    assert!(ctrl.pending().await.is_some(), "the capture still waits");

    // Unlock again (the saved server and email are reused) and the items are back
    let (s, v) = e.person("POST", "/api/passwords/bitwarden/unlock", Some(json!({"master_password": MASTER}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["bitwarden"]["item_count"], 4, "the 3 saved here and the moved leftover");
    assert_eq!(vault.for_origin(&team, "https://login.example.com").await.unwrap().unwrap().password, "DUMMY-BW-PW-1b");

    // Delete moves it to Bitwarden's trash and kwnt stops using it
    let (s, v) = e.person("DELETE", &format!("{uri}/{cid}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "deleted");
    assert!(vault.for_origin(&team, "https://login.example.com").await.unwrap().is_none());

    // Forget needs the master password
    let (s, _) = e.person("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": "DUMMY-wrong"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = e.person("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": MASTER}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["store"], "local");
    assert_eq!(v["bitwarden"], Value::Null);
    assert!(!e.data.join("kwnt-vault").join(bitwarden::CONFIG_FILE).exists());
}

/// Five wrong master passwords in a row on reveal lock the vault.
#[tokio::test]
async fn wrong_master_passwords_lock_the_vault() {
    let Some(server) = server() else { return };
    let email = new_account(&server).await;
    let e = setup().await;
    let team = e.team("bwlock").await;
    let (s, v) = e
        .person("POST", "/api/passwords/bitwarden/unlock", Some(json!({"server_url": server, "email": email, "master_password": MASTER})))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let uri = format!("/api/sandbox/{team}/passwords");
    let (_, v) = e.person("POST", &uri, Some(json!({"origin": "https://a.example.com", "username": "u", "password": "DUMMY-LOCKOUT"}))).await;
    let reveal = format!("{uri}/{}/reveal", v["credential"]["id"].as_str().unwrap());
    for _ in 0..bitwarden::MAX_WRONG_PASSWORDS - 1 {
        let (s, _) = e.person("POST", &reveal, Some(json!({"master_password": "DUMMY-guess"}))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // A right one resets the count
    let (s, _) = e.person("POST", &reveal, Some(json!({"master_password": MASTER}))).await;
    assert_eq!(s, StatusCode::OK);
    for _ in 0..bitwarden::MAX_WRONG_PASSWORDS - 1 {
        let (s, _) = e.person("POST", &reveal, Some(json!({"master_password": "DUMMY-guess"}))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, v) = e.person("POST", &reveal, Some(json!({"master_password": "DUMMY-guess"}))).await;
    assert_eq!(s, StatusCode::LOCKED, "the fifth wrong one locks: {v}");
    assert!(e.state.credentials.is_locked().await);
    let (s, _) = e.person("POST", &reveal, Some(json!({"master_password": MASTER}))).await;
    assert_eq!(s, StatusCode::LOCKED, "locked until the person unlocks again");
}

/// kwnt holds the unlocked vault key in memory, so a same-user bot must not read its memory or
/// environment: `PR_SET_DUMPABLE` 0 makes `/proc/<kwnt>/{environ,mem,…}` root's.
#[cfg(target_os = "linux")]
#[test]
fn kwnt_is_not_dumpable() {
    use std::io::BufRead;
    use std::os::unix::fs::MetadataExt;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_kwnt"))
        .args(["mcp-oauth-mock-server", "--port", "0"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Running, past main's start: it prints its address once listening
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert!(line.contains("mock OAuth MCP server"), "{line}");
    let proc_dir = PathBuf::from(format!("/proc/{}", child.id()));
    let environ = std::fs::metadata(proc_dir.join("environ")).unwrap();
    let readable = std::fs::read(proc_dir.join("environ")).is_ok();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(environ.uid(), 0, "/proc/<kwnt>/environ belongs to root");
    assert!(!readable, "kwnt's environment is unreadable to its own user");
}
