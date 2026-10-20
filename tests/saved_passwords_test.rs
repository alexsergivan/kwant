//! Saved website passwords (audit 2026-10-20, F-1..F-8): each audit repro must now fail.
//!
//! - F-1/F-7: passwords live encrypted in the server's own data dir (0700 dir, 0600 files), never
//!   in a bot's folder; an old `.saved_passwords.json` moves in on start and is deleted.
//! - F-2: the password routes, answering a card and the approvals switch need the API token even
//!   from this machine (401), and a bot's turn token is refused (403).
//! - F-3: the list is metadata only, a save doesn't echo the password, reveal is explicit.
//! - F-4: an id from another space is a 404; names don't resolve.
//! - F-5/F-6: a captured password waits for the person's confirmation (browser side in
//!   browser_test.rs).
//! - Bots can't name kwnt's secret files in a tool call, approvals on or off.

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{NewKwant, NewTeam},
        spaces::{NewSpace, SpaceId},
        Database,
    },
    router::LayaRouter,
    sandbox::{browser::BrowserController, credentials::KEY_FILE, credentials::VAULT_DIR, PendingCapture},
    server::{continuity::SelfScope, create_router, mcp_runtime, passwords, AppState},
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
    tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    Env { app: create_router(state.clone()), state, data, tmp }
}

/// A request from this machine (no remote peer), with an optional bearer.
async fn call(app: &Router, method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> (StatusCode, Value, HeaderMap) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), headers)
}

impl Env {
    fn token(&self) -> &str {
        self.state.api_token.reveal()
    }
    async fn person(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (s, v, _) = call(&self.app, method, uri, Some(self.token()), body).await;
        (s, v)
    }
    async fn first_bot(&self) -> kwnt::db::models::Kwant {
        self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0)
    }
    async fn team(&self, name: &str, workspace: Option<&Path>) -> String {
        let bot = self.first_bot().await;
        let team = NewTeam {
            name: name.into(),
            description: None,
            icon: None,
            lead_kwant_id: bot.id.clone(),
            member_kwants: vec![bot.id],
            workspace_path: workspace.map(|p| p.display().to_string()),
            repo_url: None,
        };
        self.state.db.create_team(&SpaceId::default_space(), &team).await.unwrap().id
    }
    /// A live turn of the first bot; returns its turn token.
    async fn turn_token(&self, folder: &Path) -> String {
        let bot = self.first_bot().await;
        let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
        let (_, token) = mcp_runtime::prepare_turn_with_self(&self.state, &bot, None, Some(folder.to_path_buf()), Some(scope))
            .await
            .unwrap();
        token
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

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// F-2: `curl localhost/api/sandbox/<id>/passwords` with no token returned every password, and a
/// bare curl could answer an approval card or switch approvals off.
#[tokio::test]
async fn f2_person_only_routes_refuse_no_token_and_bot_tokens() {
    let e = setup().await;
    let team = e.team("vaultteam", None).await;
    let bot = e.first_bot().await;
    let (s, v) = e.person("POST", &format!("/api/sandbox/{team}/passwords"), Some(json!({"origin": "https://example.com", "username": "alice", "password": "DUMMY-F2-PW"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let cid = v["credential"]["id"].as_str().unwrap().to_string();

    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;
    let routes: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", format!("/api/sandbox/{team}/passwords"), None),
        ("POST", format!("/api/sandbox/{team}/passwords"), Some(json!({"origin": "https://x.test", "username": "u", "password": "DUMMY-X"}))),
        ("POST", format!("/api/sandbox/{team}/passwords/{cid}/reveal"), None),
        ("DELETE", format!("/api/sandbox/{team}/passwords/{cid}"), None),
        ("POST", format!("/api/sandbox/{team}/passwords/pending"), None),
        ("DELETE", format!("/api/sandbox/{team}/passwords/pending"), None),
        ("POST", "/api/approvals/some-ticket/resolve".to_string(), Some(json!({"decision": "allow_once"}))),
        ("PUT", "/api/spaces/spc_default/approvals".to_string(), Some(json!({"approvals": "off"}))),
        ("PUT", format!("/api/kwants/{}/approvals", bot.id), Some(json!({"approvals": "off"}))),
    ];
    for (method, uri, body) in &routes {
        let (s, v, _) = call(&e.app, method, uri, None, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} without a token: {v}");
        assert_eq!(v["person_only"], true, "{v}");
        assert!(!v.to_string().contains("DUMMY"), "{v}");
        let (s, v, _) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
        let (s, _, _) = call(&e.app, method, uri, Some("not-the-token"), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a wrong token");
    }
    // Nothing changed: the password is still there and approvals are still on
    let (_, v) = e.person("GET", &format!("/api/sandbox/{team}/passwords"), None).await;
    assert_eq!(v["passwords"].as_array().unwrap().len(), 1, "{v}");
    let (_, v) = e.person("GET", "/api/spaces/spc_default/approvals", None).await;
    assert_eq!(v["approvals"], "ask");

    // The rest of the local app still works without a token, and reading the switch is open
    let (s, _, _) = call(&e.app, "GET", "/api/kwants", None, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = call(&e.app, "GET", "/api/spaces/spc_default/approvals", None, None).await;
    assert_eq!(s, StatusCode::OK);
    // The UI learns it needs the token for these
    let (_, v, _) = call(&e.app, "GET", "/api/auth/status", None, None).await;
    assert_eq!(v["person_auth_required"], true);
    assert_eq!(v["person_authenticated"], false);
    let (_, v) = e.person("GET", "/api/auth/status", None).await;
    assert_eq!(v["person_authenticated"], true);
}

/// F-3: the list returned every password in clear and a save echoed it back.
#[tokio::test]
async fn f3_list_is_metadata_only_and_reveal_is_explicit() {
    let e = setup().await;
    let team = e.team("metateam", None).await;
    let (s, v) = e.person("POST", &format!("/api/sandbox/{team}/passwords"), Some(json!({"domain": "example.com", "username": "alex", "password": "DUMMY-F3-PW"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains("DUMMY-F3-PW"), "save echoed the password: {v}");
    assert_eq!(v["credential"]["origin"], "https://example.com");
    let id = v["credential"]["id"].as_str().unwrap().to_string();

    let (_, v) = e.person("GET", &format!("/api/sandbox/{team}/passwords"), None).await;
    assert!(!v.to_string().contains("DUMMY-F3-PW"), "list has the password: {v}");
    let item = &v["passwords"][0];
    let mut keys: Vec<&str> = item.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["created_at", "domain", "id", "last_used", "origin", "username"]);
    assert_eq!(item["domain"], "example.com");
    assert_eq!(v["pending"], Value::Null);

    let (s, v, headers) = call(&e.app, "POST", &format!("/api/sandbox/{team}/passwords/{id}/reveal"), Some(e.token()), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({"password": "DUMMY-F3-PW"}));
    assert_eq!(headers["cache-control"], "no-store");
    let (s, _) = e.person("POST", &format!("/api/sandbox/{team}/passwords/nope/reveal"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Bad input is refused without echoing anything
    for body in [json!({"origin": "javascript:alert(1)", "username": "a", "password": "DUMMY-BAD"}), json!({"origin": "https://a.test", "username": "a", "password": ""})] {
        let (s, v) = e.person("POST", &format!("/api/sandbox/{team}/passwords"), Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        assert!(!v.to_string().contains("DUMMY"));
    }
    let (_, v) = e.person("DELETE", &format!("/api/sandbox/{team}/passwords/{id}"), None).await;
    assert_eq!(v["status"], "deleted");
    let (_, v) = e.person("DELETE", &format!("/api/sandbox/{team}/passwords/{id}"), None).await;
    assert_eq!(v["status"], "not_found");
}

/// F-1/F-7: a team with a project folder kept `.saved_passwords.json` in clear, 0644, inside the
/// bots' own folder.
#[tokio::test]
async fn f1_f7_passwords_leave_bot_folders_encrypted_and_private() {
    let e = setup().await;
    let proj = e.tmp.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let team = e.team("projteam", Some(&proj)).await;
    let legacy = json!([
        {"id": "1", "domain": "example.com", "origin": "https://example.com", "username": "bob", "password": "DUMMY-PROJ-PW-9911", "created_at": "2026-01-01T00:00:00Z", "last_used": null},
        {"id": "2", "domain": "old.test", "origin": "", "username": "carol", "password": "DUMMY-OLD-PW-2"}
    ]);
    std::fs::write(proj.join(".saved_passwords.json"), legacy.to_string()).unwrap();
    // A bot's own browser folder too, picked up on its first use
    let bot = e.first_bot().await;
    let bot_folder = kwnt::server::routes::resolve_sandbox_target_info(&e.state, &bot.id).await.unwrap().1;
    std::fs::create_dir_all(&bot_folder).unwrap();
    std::fs::write(bot_folder.join(".saved_passwords.json"), json!([{"domain": "bot.test", "username": "dan", "password": "DUMMY-BOT-PW-3"}]).to_string()).unwrap();

    passwords::migrate_all(&e.state).await;

    assert!(!proj.join(".saved_passwords.json").exists(), "the old file is deleted");
    assert!(!bot_folder.join(".saved_passwords.json").exists());
    let (_, v) = e.person("GET", &format!("/api/sandbox/{team}/passwords"), None).await;
    let origins: Vec<&str> = v["passwords"].as_array().unwrap().iter().map(|p| p["origin"].as_str().unwrap()).collect();
    assert_eq!(origins.len(), 2, "{v}");
    assert!(origins.contains(&"https://example.com") && origins.contains(&"https://old.test"), "{v}");
    let (_, v) = e.person("GET", &format!("/api/sandbox/{}/passwords", bot.id), None).await;
    assert_eq!(v["passwords"][0]["username"], "dan", "{v}");

    // Nowhere in clear: not in the project, not in the data dir
    for needle in ["DUMMY-PROJ-PW-9911", "DUMMY-OLD-PW-2", "DUMMY-BOT-PW-3"] {
        no_file_contains(&proj, needle);
        no_file_contains(&e.data, needle);
    }
    // In the server's own data dir, outside every bot folder, private
    let vault = e.data.join(VAULT_DIR);
    let files = files_under(&vault);
    assert!(!files.is_empty());
    assert!(!vault.starts_with(&proj) && !vault.starts_with(&bot_folder));
    #[cfg(unix)]
    {
        assert_eq!(mode(&vault), 0o700);
        assert_eq!(mode(&e.data.join(KEY_FILE)), 0o600);
        for f in files {
            assert_eq!(mode(&f), 0o600, "{}", f.display());
        }
    }
}

/// F-4: `GET /api/sandbox/<bot in space B>/passwords?space=spc_default` returned B's passwords,
/// and a team's name resolved like its id.
#[tokio::test]
async fn f4_other_spaces_and_names_are_not_found() {
    let e = setup().await;
    let lab = e.state.db.create_space(&NewSpace { name: "Lab".into(), icon: "🧪".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let lab_bot = e
        .state
        .db
        .create_kwant(
            &SpaceId::new(&lab.id),
            &NewKwant {
                name: "Bbot".into(),
                title: "t".into(),
                persona_prompt: "p".into(),
                soul: String::new(),
                avatar_sprite: "architect".into(),
                driver_type: "api".into(),
                preferred_model: "auto".into(),
            },
        )
        .await
        .unwrap();
    let (s, _) = e.person("POST", &format!("/api/sandbox/{}/passwords", lab_bot.id), Some(json!({"origin": "https://zed.test", "username": "zed", "password": "DUMMY-SPACEB-PW-1"}))).await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = e.person("GET", &format!("/api/sandbox/{}/passwords?space=spc_default", lab_bot.id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = e.person("GET", &format!("/api/sandbox/{}/passwords?space={}", lab_bot.id, lab.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["passwords"][0]["username"], "zed");

    // A channel of the default space asked for in Lab
    let general = e.state.db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    let (s, _) = e.person("GET", &format!("/api/sandbox/{}/passwords?space={}", general.id, lab.id), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("GET", &format!("/api/sandbox/{}/passwords?space=spc_default", general.id), None).await;
    assert_eq!(s, StatusCode::OK);

    // Ids only: a team's name (the seeded core-engineering team) is not a password owner
    let (s, _) = e.person("GET", "/api/sandbox/core-engineering/passwords", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("GET", "/api/sandbox/no-such-id/passwords", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// F-6: a captured password was saved on the next click with no prompt. Now it waits.
#[tokio::test]
async fn f6_a_captured_password_waits_for_the_person() {
    let e = setup().await;
    let team = e.team("captureteam", None).await;
    let ctrl = BrowserController::new(9);
    e.state.browser_controllers.lock().await.insert(team.clone(), ctrl.clone());
    let capture = PendingCapture {
        origin: "https://login.example.com".into(),
        username: "carol".into(),
        password: "DUMMY-CAPTURED-PW".into(),
        captured_at: "2026-10-20T00:00:00Z".into(),
    };
    ctrl.hold_for_confirmation(capture.clone()).await;

    let uri = format!("/api/sandbox/{team}/passwords");
    let (_, v) = e.person("GET", &uri, None).await;
    assert!(v["passwords"].as_array().unwrap().is_empty(), "nothing saved before the person says so: {v}");
    assert_eq!(v["pending"]["origin"], "https://login.example.com");
    assert_eq!(v["pending"]["username"], "carol");
    assert!(!v.to_string().contains("DUMMY-CAPTURED-PW"), "{v}");

    // Discard drops it
    let (_, v) = e.person("DELETE", &format!("{uri}/pending"), None).await;
    assert_eq!(v["status"], "discarded");
    let (s, _) = e.person("POST", &format!("{uri}/pending"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Save keeps it under the origin the browser saw
    ctrl.hold_for_confirmation(capture).await;
    let (s, v) = e.person("POST", &format!("{uri}/pending"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("DUMMY-CAPTURED-PW"));
    assert_eq!(v["credential"]["origin"], "https://login.example.com");
    let (_, v) = e.person("GET", &uri, None).await;
    assert_eq!(v["passwords"].as_array().unwrap().len(), 1);
    assert_eq!(v["pending"], Value::Null);
}

/// What claude's permission prompt gets back for a built-in tool call of a live turn.
async fn claude_asks(e: &Env, turn: &str, tool: &str, input: Value) -> String {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
    let (s, v, _) = call(&e.app, "POST", &format!("/api/mcp/permission/{turn}"), None, Some(rpc)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["result"]["content"][0]["text"].as_str().unwrap().to_string()
}

/// With approvals off a bot could Read the plaintext file (F-1). The vault is encrypted now, and
/// a tool call that names the vault, its key or the API token is refused outright.
#[tokio::test]
async fn bots_cannot_name_kwnt_secret_files_even_with_approvals_off() {
    let e = setup().await;
    let bot = e.first_bot().await;
    let (s, _) = e.person("PUT", &format!("/api/kwants/{}/approvals", bot.id), Some(json!({"approvals": "off"}))).await;
    assert_eq!(s, StatusCode::OK);
    let team = e.team("t", None).await;
    e.person("POST", &format!("/api/sandbox/{team}/passwords"), Some(json!({"origin": "https://a.test", "username": "u", "password": "DUMMY-OFF-PW"}))).await;
    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;

    let key = e.data.join(KEY_FILE).display().to_string();
    let vault = e.data.join(VAULT_DIR).display().to_string();
    let token_file = e.data.join("api-token").display().to_string();
    #[cfg(unix)]
    std::os::unix::fs::symlink(e.data.join(KEY_FILE), folder.path().join("notes.txt")).unwrap();
    let refused = [
        ("Read", json!({"file_path": key})),
        ("Read", json!({"file_path": format!("{vault}/x.bin")})),
        ("Bash", json!({"command": format!("cat {token_file}")})),
        ("Bash", json!({"command": "kwnt token"})),
        ("Grep", json!({"pattern": ".", "path": vault})),
        ("Read", json!({"file_path": folder.path().join("notes.txt").display().to_string()})),
        ("Read", json!({"file_path": "notes.txt"})),
    ];
    for (tool, input) in refused {
        let answer = claude_asks(&e, &turn, tool, input.clone()).await;
        assert!(answer.contains(r#""behavior":"deny""#), "{tool} {input}: {answer}");
    }
    let answer = claude_asks(&e, &turn, "Bash", json!({"command": "ls"})).await;
    assert!(answer.contains(r#""behavior":"allow""#), "approvals off still runs ordinary calls: {answer}");

    let (_, denied) = e.person("GET", "/api/approvals?status=denied", None).await;
    assert_eq!(denied.as_array().map(Vec::len), Some(7), "refusals are recorded: {denied}");
}
