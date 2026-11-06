//! Approvals off and the built-in password store don't mix (audit 2026-10-20, F-10): a bot with
//! approvals off could read the built-in store's key and the API token from kwnt's data folder,
//! and with them every built-in saved password.
//!
//! - Switching approvals off (space, bot, or a bot set to inherit an off space) answers 409
//!   `{error, needs_bitwarden: true, local_count}` while the built-in store holds any password,
//!   in Bitwarden mode too (leftovers open with the built-in key).
//! - While any space or bot is on "Ask only when risky" or "Don't ask", saving into the built-in store answers 409 and going back
//!   from Bitwarden to the built-in store answers 409 unless it is empty.
//! - An install already in that state shows `exposed: true` and keeps its approvals.
//! - 0.7.3 A1: archived spaces, and bots in them, don't count. A2: the person (never a bot's turn
//!   token) can accept the risk once (`accept_risk: true`), and then none of these refuse.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, NewTeam},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    sandbox::{bitwarden::BitwardenConfig, bitwarden::CONFIG_FILE, browser::BrowserController, credentials::VAULT_DIR, PendingCapture},
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    team: String,
    _tmp: TempDir,
}

/// A fresh server; with `bitwarden`, Bitwarden is set up (and locked, as after a restart).
async fn setup(bitwarden: bool) -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    if bitwarden {
        let dir = data.join(VAULT_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cfg = BitwardenConfig {
            server_url: "https://vault.bitwarden.test".into(),
            email: "dummy@example.test".into(),
            client_id: None,
            device_id: "d".into(),
        };
        std::fs::write(dir.join(CONFIG_FILE), serde_json::to_vec(&cfg).unwrap()).unwrap();
    }
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    assert_eq!(state.credentials.store_name(), if bitwarden { "bitwarden" } else { "local" });
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let new_team = NewTeam {
        name: "t".into(),
        description: None,
        icon: None,
        lead_kwant_id: bot.id.clone(),
        member_kwants: vec![bot.id.clone()],
        workspace_path: None,
        repo_url: None,
    };
    let team = state.db.create_team(&SpaceId::default_space(), &new_team).await.unwrap().id;
    Env { app: create_router(state.clone()), state, bot, team, _tmp: tmp }
}

impl Env {
    async fn person(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.state.api_token.reveal()));
        let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
        let resp = self.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    async fn space_approvals(&self, mode: &str) -> (StatusCode, Value) {
        let uri = format!("/api/spaces/{}/approvals", SpaceId::default_space().as_str());
        self.person("PUT", &uri, Some(json!({"approvals": mode}))).await
    }
    async fn bot_approvals(&self, mode: &str) -> (StatusCode, Value) {
        self.person("PUT", &format!("/api/kwants/{}/approvals", self.bot.id), Some(json!({"approvals": mode}))).await
    }
    async fn save(&self, password: &str) -> (StatusCode, Value) {
        let body = json!({"origin": "https://a.test", "username": "u", "password": password});
        self.person("POST", &format!("/api/sandbox/{}/passwords", self.team), Some(body)).await
    }
    /// A password straight into the built-in store, as an older install left it.
    async fn local_password(&self) {
        self.state.credentials.local().save(&self.team, "https://old.test", "u", "DUMMY-LOCAL-PW").await.unwrap();
    }
    async fn store(&self) -> Value {
        self.person("GET", "/api/passwords/store", None).await.1
    }
    async fn effective(&self) -> String {
        self.state.db.kwant_approvals(&self.bot.id).await.unwrap().effective
    }
}

fn assert_needs_bitwarden(s: StatusCode, v: &Value, count: Option<u64>) {
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_bitwarden"], true, "{v}");
    let e = v["error"].as_str().unwrap_or_default();
    // Names the mode it means: "approvals are off" read wrong under "Ask only when risky"
    assert!(e.contains("Ask only when risky"), "{v}");
    assert!(!e.contains("approvals are off"), "{v}");
    if let Some(n) = count {
        assert_eq!(v["local_count"], n, "{v}");
    }
    // 0.7.3 A2: the person may accept the risk instead
    assert_eq!(v["can_accept"], true, "{v}");
    assert!(!v.to_string().contains("DUMMY"), "no password in a refusal: {v}");
}

#[tokio::test]
async fn approvals_off_is_refused_while_the_built_in_store_holds_passwords() {
    let e = setup(false).await;
    let (s, v) = e.save("DUMMY-PW-1").await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, v) = e.space_approvals("off").await;
    assert_needs_bitwarden(s, &v, Some(1));
    let (s, v) = e.bot_approvals("off").await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert_eq!(e.effective().await, "ask", "nothing switched off");
    assert_eq!(e.state.db.space_approvals(&SpaceId::default_space()).await.unwrap(), "ask");

    // Approvals on stays allowed
    assert_eq!(e.space_approvals("ask").await.0, StatusCode::OK);
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
    assert_eq!(e.bot_approvals("inherit").await.0, StatusCode::OK, "inherit of a space that asks");
}

#[tokio::test]
async fn inheriting_an_off_space_counts_as_switching_off() {
    let e = setup(false).await;
    // The bot asks on its own while the space goes off, with nothing saved yet
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
    assert_eq!(e.space_approvals("off").await.0, StatusCode::OK);
    // A password from before (an older install): the bot can't be moved onto the space's off
    e.local_password().await;
    let (s, v) = e.bot_approvals("inherit").await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert_eq!(e.effective().await, "ask");
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
}

#[tokio::test]
async fn approvals_off_is_allowed_with_an_empty_built_in_store() {
    let e = setup(false).await;
    let (s, v) = e.space_approvals("off").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = e.bot_approvals("off").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(e.effective().await, "off");
    let st = e.store().await;
    assert_eq!(st["approvals_off"], true, "{st}");
    assert_eq!(st["exposed"], false, "{st}");
}

#[tokio::test]
async fn with_approvals_off_nothing_new_is_saved_in_the_built_in_store() {
    for scope in ["space", "bot", "inherit"] {
        let e = setup(false).await;
        match scope {
            "space" => assert_eq!(e.space_approvals("off").await.0, StatusCode::OK),
            "bot" => assert_eq!(e.bot_approvals("off").await.0, StatusCode::OK),
            _ => {
                // A space that is off, its bots overriding to ask: still off for new bots
                assert_eq!(e.space_approvals("off").await.0, StatusCode::OK);
                for k in e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap() {
                    e.state.db.set_kwant_approvals(&k.id, "ask").await.unwrap();
                }
            }
        }
        let (s, v) = e.save("DUMMY-PW-OFF").await;
        assert_needs_bitwarden(s, &v, None);
        // Why, and both ways out, in the person's words
        let why = v["error"].as_str().unwrap();
        for part in ["Ask before running commands", "Don't ask", "Connect Bitwarden", "switch every space and bot"] {
            assert!(why.contains(part), "{scope}: {part}: {why}");
        }
        assert_eq!(e.store().await["local_count"], 0, "{scope}: nothing saved");

        // A password the browser captured keeps waiting
        let ctrl = BrowserController::new(9);
        e.state.browser_controllers.lock().await.insert(e.team.clone(), ctrl.clone());
        ctrl.hold_for_confirmation(PendingCapture {
            origin: "https://login.example.test".into(),
            username: "carol".into(),
            password: "DUMMY-CAPTURED-PW".into(),
            captured_at: "2026-10-20T00:00:00Z".into(),
        })
        .await;
        let (s, v) = e.person("POST", &format!("/api/sandbox/{}/passwords/pending", e.team), None).await;
        assert_needs_bitwarden(s, &v, None);
        let (_, list) = e.person("GET", &format!("/api/sandbox/{}/passwords", e.team), None).await;
        assert_eq!(list["pending"]["username"], "carol", "{scope}: still waiting: {list}");
        assert_eq!(list["passwords"].as_array().map(Vec::len), Some(0));

        // Approvals back on: saving works again
        assert_eq!(e.space_approvals("ask").await.0, StatusCode::OK);
        assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
        let (s, v) = e.save("DUMMY-PW-ON").await;
        assert_eq!(s, StatusCode::OK, "{scope}: {v}");
    }
}

#[tokio::test]
async fn bitwarden_mode_allows_approvals_off_unless_built_in_leftovers_remain() {
    let e = setup(true).await;
    let (s, v) = e.space_approvals("off").await;
    assert_eq!(s, StatusCode::OK, "Bitwarden with an empty built-in store: {v}");
    // Saving goes to Bitwarden, not refused for approvals (locked here: 423)
    let (s, v) = e.save("DUMMY-PW-BW").await;
    assert_eq!(s, StatusCode::LOCKED, "{v}");
    assert_eq!(e.space_approvals("ask").await.0, StatusCode::OK);

    // Passwords never moved out of the built-in store open with its key: refused too
    e.local_password().await;
    let (s, v) = e.space_approvals("off").await;
    assert_needs_bitwarden(s, &v, Some(1));
    let (s, v) = e.bot_approvals("off").await;
    assert_needs_bitwarden(s, &v, Some(1));
}

#[tokio::test]
async fn going_back_to_the_built_in_store_is_refused_while_approvals_are_off() {
    let e = setup(true).await;
    assert_eq!(e.bot_approvals("off").await.0, StatusCode::OK);
    // Leftovers appear in the built-in store (an older install)
    e.local_password().await;
    let st = e.store().await;
    assert_eq!(st["exposed"], true, "{st}");

    let (s, v) = e.person("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": "DUMMY-MASTER"}))).await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert_eq!(e.state.credentials.store_name(), "bitwarden", "still Bitwarden");

    // With approvals back on, the refusal is gone (the locked vault then asks for an unlock)
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
    let (s, v) = e.person("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": "DUMMY-MASTER"}))).await;
    assert_ne!(s, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn going_back_with_an_empty_built_in_store_is_not_refused() {
    let e = setup(true).await;
    assert_eq!(e.space_approvals("off").await.0, StatusCode::OK);
    let (s, v) = e.person("DELETE", "/api/passwords/bitwarden", Some(json!({"master_password": "DUMMY-MASTER"}))).await;
    assert_ne!(s, StatusCode::CONFLICT, "nothing to expose: {v}");
}

#[tokio::test]
async fn an_install_already_exposed_is_reported_and_left_as_is() {
    let e = setup(false).await;
    let st = e.store().await;
    assert_eq!((st["approvals_off"].clone(), st["exposed"].clone()), (json!(false), json!(false)), "{st}");

    // Approvals were switched off before this release, then a password was saved
    e.state.db.set_kwant_approvals(&e.bot.id, "off").await.unwrap();
    e.local_password().await;
    let st = e.store().await;
    assert_eq!(st["store"], "local");
    assert_eq!(st["local_count"], 1);
    assert_eq!(st["approvals_off"], true);
    assert_eq!(st["exposed"], true, "{st}");
    assert!(!st.to_string().contains("DUMMY"), "{st}");
    kwnt::server::passwords::warn_if_exposed(&e.state).await;
    assert_eq!(e.effective().await, "off", "approvals are not changed silently");

    // Turning approvals back on clears it
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
    assert_eq!(e.store().await["exposed"], false);
}

#[tokio::test]
async fn approvals_off_of_a_deleted_bot_does_not_count() {
    let e = setup(false).await;
    e.state.db.set_kwant_approvals("no-such-bot", "off").await.unwrap();
    assert!(!e.state.db.any_approvals_off().await.unwrap());
    let (s, v) = e.save("DUMMY-PW-2").await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn ask_only_when_risky_follows_the_same_rule_as_off() {
    let e = setup(false).await;
    e.local_password().await;
    let (s, v) = e.space_approvals("smart").await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert!(v["error"].as_str().unwrap().contains("Ask only when risky"), "{v}");
    let (s, v) = e.bot_approvals("smart").await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert_eq!(e.effective().await, "ask", "nothing relaxed");

    // With an empty store smart is allowed, and then saving into the built-in store is refused
    let e = setup(false).await;
    assert_eq!(e.bot_approvals("smart").await.0, StatusCode::OK);
    assert_eq!(e.effective().await, "smart");
    assert_eq!(e.store().await["approvals_off"], true, "smart counts as off");
    let (s, v) = e.save("DUMMY-PW-2").await;
    assert_needs_bitwarden(s, &v, None);
    // A bot inheriting a smart space counts too
    let e = setup(false).await;
    assert_eq!(e.bot_approvals("ask").await.0, StatusCode::OK);
    assert_eq!(e.space_approvals("smart").await.0, StatusCode::OK);
    e.local_password().await;
    let (s, v) = e.bot_approvals("inherit").await;
    assert_needs_bitwarden(s, &v, Some(1));
}

// ---- 0.7.3 A1 and A2 (github-connect-073/spec.md) ----

use kwnt::{
    db::{models::NewKwant, spaces::NewSpace},
    server::{
        continuity::{SelfScope, TurnOrigin},
        mcp_runtime,
        passwords::RISK_ACCEPTED,
    },
};

impl Env {
    async fn as_bot(&self, token: &str, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"));
        let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
        let resp = self.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    /// A bot's turn token in #general, as a running turn holds it.
    async fn bot_token(&self) -> String {
        let space = SpaceId::default_space();
        let general = self.state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
        let scope = SelfScope {
            kwant_id: self.bot.id.clone(),
            kwant_name: self.bot.name.clone(),
            origin: Some(TurnOrigin { channel_id: Some(general.id.clone()), ..Default::default() }),
            turn_id: Some("t-risk".into()),
            ..Default::default()
        };
        mcp_runtime::prepare_turn_with_self(&self.state, &self.bot, Some("t-risk".into()), None, Some(scope)).await.unwrap().1
    }
    async fn space(&self, approvals: &str) -> SpaceId {
        let input = NewSpace { name: "Old".into(), icon: "x".into(), color: None, this_machine_enabled: true };
        SpaceId::new(self.state.db.create_space_with_approvals(&input, approvals).await.unwrap().id)
    }
    async fn accepted(&self) -> Option<String> {
        self.state.db.get_setting(RISK_ACCEPTED).await.unwrap().filter(|v| !v.is_empty())
    }
    async fn archived(&self, space: &SpaceId) -> bool {
        self.state.db.get_space(space).await.unwrap().unwrap().archived_at.is_some()
    }
}

#[tokio::test]
async fn an_archived_space_on_ask_only_when_risky_does_not_count() {
    // Oleks's dead end: an archived space on smart refused every save, with no way to see why
    let e = setup(false).await;
    let old = e.space("smart").await;
    assert_eq!(e.store().await["approvals_off"], true);
    let (s, v) = e.save("DUMMY-PW-A1").await;
    assert_needs_bitwarden(s, &v, None);

    let (s, v) = e.person("PATCH", &format!("/api/spaces/{}", old.as_str()), Some(json!({"archived": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.state.db.any_approvals_off().await.unwrap());
    let st = e.store().await;
    assert_eq!((st["approvals_off"].clone(), st["risk_accepted"].clone()), (json!(false), json!(false)), "{st}");
    let (s, v) = e.save("DUMMY-PW-A1").await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Restoring it would bring the risk back: the same 409, nothing restored
    let uri = format!("/api/spaces/{}", old.as_str());
    let (s, v) = e.person("PATCH", &uri, Some(json!({"archived": false}))).await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert!(e.archived(&old).await, "still archived");
    // A bot can't accept it on the person's behalf
    let token = e.bot_token().await;
    let (s, v) = e.as_bot(&token, "PATCH", &uri, Some(json!({"archived": false, "accept_risk": true}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(e.archived(&old).await);
    assert_eq!(e.accepted().await, None);
    // The person accepts and restores
    let (s, v) = e.person("PATCH", &uri, Some(json!({"archived": false, "accept_risk": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.archived(&old).await);
    let st = e.store().await;
    assert_eq!(st["risk_accepted"], true, "{st}");
    assert_eq!(st["exposed"], true, "accepting doesn't hide the exposure: {st}");
}

#[tokio::test]
async fn restoring_a_space_that_asks_is_never_refused() {
    let e = setup(false).await;
    e.local_password().await;
    let old = e.space("ask").await;
    let uri = format!("/api/spaces/{}", old.as_str());
    assert_eq!(e.person("PATCH", &uri, Some(json!({"archived": true}))).await.0, StatusCode::OK);
    let (s, v) = e.person("PATCH", &uri, Some(json!({"archived": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(e.accepted().await, None, "nothing accepted");
}

#[tokio::test]
async fn a_bot_override_in_an_archived_space_does_not_count() {
    let e = setup(false).await;
    let old = e.space("ask").await;
    let input = NewKwant {
        name: "Oldie".into(),
        title: "t".into(),
        persona_prompt: "p".into(),
        soul: String::new(),
        avatar_sprite: "architect".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    let oldie = e.state.db.create_kwant(&old, &input).await.unwrap();
    e.state.db.set_kwant_approvals(&oldie.id, "smart").await.unwrap();
    assert!(e.state.db.any_approvals_off().await.unwrap());
    e.state.db.archive_space(&old).await.unwrap();
    assert!(!e.state.db.any_approvals_off().await.unwrap());
    let (s, v) = e.save("DUMMY-PW-A1B").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Restoring it is refused while the built-in store holds that password
    let uri = format!("/api/spaces/{}", old.as_str());
    let (s, v) = e.person("PATCH", &uri, Some(json!({"archived": false}))).await;
    assert_needs_bitwarden(s, &v, Some(1));
}

#[tokio::test]
async fn accepting_the_risk_on_save_lets_every_later_step_through() {
    let e = setup(false).await;
    assert_eq!(e.bot_approvals("smart").await.0, StatusCode::OK);
    let (s, v) = e.save("DUMMY-PW-R1").await;
    assert_needs_bitwarden(s, &v, None);
    assert_eq!(e.store().await["local_count"], 0);

    let body = json!({"origin": "https://a.test", "username": "u", "password": "DUMMY-PW-R1", "accept_risk": true});
    let (s, v) = e.person("POST", &format!("/api/sandbox/{}/passwords", e.team), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let at = e.accepted().await.expect("acceptance saved");
    assert!(chrono::DateTime::parse_from_rfc3339(&at).is_ok(), "{at}");
    let st = e.store().await;
    assert_eq!(st["risk_accepted"], true, "{st}");
    assert_eq!(st["local_count"], 1);
    assert_eq!(st["exposed"], true, "{st}");
    assert!(!st.to_string().contains("DUMMY"), "{st}");
    kwnt::server::passwords::warn_if_exposed(&e.state).await;

    // Later saves and relaxing approvals need no flag
    let (s, v) = e.save("DUMMY-PW-R2").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = e.space_approvals("off").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(e.state.db.space_approvals(&SpaceId::default_space()).await.unwrap(), "off");
    // Accepted once: a second acceptance keeps the first time
    let (s, v) = e.person("POST", "/api/passwords/store/accept_risk", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(e.accepted().await.as_deref(), Some(at.as_str()));
}

#[tokio::test]
async fn the_accept_route_and_the_approvals_flag_accept_too() {
    let e = setup(false).await;
    e.local_password().await;
    let (s, v) = e.person("POST", "/api/passwords/store/accept_risk", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["risk_accepted"], true, "the store reply: {v}");
    assert_eq!(v["store"], "local", "{v}");
    assert_eq!(e.space_approvals("smart").await.0, StatusCode::OK);

    let e = setup(false).await;
    e.local_password().await;
    let uri = format!("/api/spaces/{}/approvals", SpaceId::default_space().as_str());
    let (s, v) = e.person("PUT", &uri, Some(json!({"approvals": "smart", "accept_risk": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(e.accepted().await.is_some());
    // Without the flag nothing is accepted: a plain refusal
    let e = setup(false).await;
    e.local_password().await;
    let (s, v) = e.person("PUT", &uri, Some(json!({"approvals": "smart", "accept_risk": false}))).await;
    assert_needs_bitwarden(s, &v, Some(1));
    assert_eq!(e.accepted().await, None);
}

#[tokio::test]
async fn a_bot_turn_token_can_never_accept_the_risk() {
    let e = setup(false).await;
    e.local_password().await;
    let token = e.bot_token().await;
    let save = json!({"origin": "https://a.test", "username": "u", "password": "DUMMY-PW-BOT", "accept_risk": true});
    let space = format!("/api/spaces/{}/approvals", SpaceId::default_space().as_str());
    let bot = format!("/api/kwants/{}/approvals", e.bot.id);
    let calls = [
        ("POST", "/api/passwords/store/accept_risk".to_string(), None),
        ("POST", format!("/api/sandbox/{}/passwords", e.team), Some(save)),
        ("POST", format!("/api/sandbox/{}/passwords/pending", e.team), Some(json!({"accept_risk": true}))),
        ("PUT", space, Some(json!({"approvals": "off", "accept_risk": true}))),
        ("PUT", bot, Some(json!({"approvals": "smart", "accept_risk": true}))),
        ("DELETE", "/api/passwords/bitwarden".to_string(), Some(json!({"accept_risk": true}))),
        ("POST", "/api/zombie/sources".to_string(), Some(json!({"kind": "github", "accept_risk": true}))),
    ];
    for (method, uri, body) in calls {
        let (s, v) = e.as_bot(&token, method, &uri, body).await;
        // The person-only routes stop it at the door (401/403); the rest in accept_risk (403)
        assert!(matches!(s, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED), "{method} {uri}: {s} {v}");
        assert_eq!(e.accepted().await, None, "{method} {uri}");
    }
    assert_eq!(e.effective().await, "ask", "nothing relaxed");
    assert_eq!(e.store().await["local_count"], 1, "nothing saved");
    assert_eq!(e.store().await["risk_accepted"], false);
}
