//! Spaces S2: spaces through the API. A second space is made with `POST /api/spaces`, filled
//! through `?space=`, and every collection is checked to show only its own space; archive and
//! restore, the Default archive refusal and team folder collisions are covered too. A fake claude
//! CLI on PATH answers turns, so no real engine is ever called. Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use futures::StreamExt;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer ok"}'
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

struct Env {
    app: Router,
    state: AppState,
    data: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
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
    let app = create_router(state.clone());
    Env { app, state, data, _tmp: tmp }
}

fn bot(name: &str) -> Value {
    json!({
        "name": name,
        "title": format!("{name} the tester"),
        "persona_prompt": "Test persona",
        "avatar_sprite": "s",
        "driver_type": "api",
        "preferred_model": "claude",
    })
}

fn team(name: &str, lead: &str, members: &[&str], workspace_path: Option<&str>) -> Value {
    json!({"name": name, "lead_kwant_id": lead, "member_kwants": members, "workspace_path": workspace_path})
}

fn routine(kwant_id: &str, name: &str, enabled: bool) -> Value {
    json!({
        "kwant_id": kwant_id,
        "name": name,
        "instructions": "Summarise yesterday.",
        "schedule": {"type": "interval", "every_minutes": 60},
        "enabled": enabled,
    })
}

impl Env {
    async fn new_space(&self, name: &str, this_machine: bool) -> String {
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/spaces",
            Some(json!({"name": name, "icon": "🧪", "this_machine_enabled": this_machine})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    async fn space(&self, id: &str) -> Value {
        ok(&self.app, "GET", &format!("/api/spaces/{id}"), None).await
    }
}

async fn next_event<S, E>(stream: &mut S) -> Value
where
    S: futures::Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
    E: std::fmt::Debug,
{
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.expect("an event").unwrap().unwrap();
    let text = String::from_utf8_lossy(&chunk).to_string();
    let data = text.lines().find_map(|l| l.strip_prefix("data:")).expect("a data line").trim().to_string();
    serde_json::from_str(&data).unwrap()
}

#[tokio::test]
async fn two_spaces_end_to_end_through_the_api() {
    let env = setup().await;
    let app = &env.app;

    // Space A (Default), filled without ?space=
    let remy = ok(app, "POST", "/api/kwants", Some(bot("Remy"))).await;
    let remy_id = remy["id"].as_str().unwrap().to_string();
    ok(app, "POST", "/api/teams", Some(team("alpha", &remy_id, &[&remy_id], None))).await;
    ok(app, "POST", "/api/routines", Some(routine(&remy_id, "alpha-digest", true))).await;
    ok(
        app,
        "POST",
        "/api/skills",
        Some(json!({"name": "alpha-notes", "description": "Alpha notes", "body": "Steps.", "kwant_id": remy_id})),
    )
    .await;
    ok(
        app,
        "POST",
        "/api/mcp/servers",
        Some(json!({"name": "alpha_mcp", "transport": "http", "url": "https://a.example.com/mcp"})),
    )
    .await;
    ok(app, "POST", "/api/profiles", Some(json!({"engine": "claude", "name": "Alpha key", "kind": "api_key"}))).await;

    // Space B through the API: seeded, empty, with its own folder
    let b = env.new_space("Side project", false).await;
    let view = env.space(&b).await;
    assert_eq!(view["name"], "Side project");
    assert_eq!(view["is_default"], false);
    assert_eq!(view["archived_at"], Value::Null);
    assert_eq!(view["counts"]["bots"], 0);
    assert_eq!(view["paused_routines"], 0);
    assert_eq!(view["general_channel_id"], Value::Null, "#general comes with the first bot");
    assert!(env.data.join("spaces").join(&b).is_dir(), "B has a work folder");
    let profiles = env.state.db.list_profiles(&SpaceId::new(&b)).await.unwrap();
    let system: Vec<_> = profiles.iter().filter(|p| p.kind == "system").collect();
    assert!(!system.is_empty() && system.iter().all(|p| !p.enabled), "this_machine_enabled: false is kept");

    let q = format!("?space={b}");
    let bea = ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Bea"))).await;
    let bea_id = bea["id"].as_str().unwrap().to_string();
    assert_eq!(bea["space_id"], b.as_str());
    let cy = ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Cy"))).await;
    let cy_id = cy["id"].as_str().unwrap().to_string();
    let view = env.space(&b).await;
    assert_eq!(view["counts"]["bots"], 2);
    let general_b = view["general_channel_id"].as_str().expect("#general after the first bot").to_string();

    // B's bots run nowhere yet: This machine is off in B and B has no account
    let kwants_b = ok(app, "GET", &format!("/api/kwants{q}"), None).await;
    let runs_on = &kwants_b.as_array().unwrap()[0]["runs_on"];
    assert_eq!(runs_on["blocked"], true, "{runs_on}");
    // A's bot runs on This machine
    let kwants_a = ok(app, "GET", "/api/kwants", None).await;
    let remy_row = kwants_a.as_array().unwrap().iter().find(|k| k["id"] == remy_id.as_str()).unwrap().clone();
    assert_eq!(remy_row["runs_on"]["account"], "This machine", "{remy_row}");

    ok(app, "POST", &format!("/api/teams{q}"), Some(team("beta", &bea_id, &[&bea_id, &cy_id], None))).await;
    ok(app, "POST", &format!("/api/routines{q}"), Some(routine(&bea_id, "beta-digest", true))).await;
    ok(
        app,
        "POST",
        &format!("/api/skills{q}"),
        Some(json!({"name": "beta-notes", "description": "Beta notes", "body": "Steps."})),
    )
    .await;
    ok(
        app,
        "POST",
        &format!("/api/mcp/servers{q}"),
        Some(json!({"name": "beta_mcp", "transport": "http", "url": "https://b.example.com/mcp"})),
    )
    .await;
    ok(
        app,
        "POST",
        &format!("/api/profiles{q}"),
        Some(json!({"engine": "claude", "name": "Beta key", "kind": "api_key"})),
    )
    .await;
    // A routine for A's bot can't be made in B
    let (s, v) = call(app, "POST", &format!("/api/routines{q}"), Some(routine(&remy_id, "stray", true))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // Every collection under ?space= shows only its own space
    let a_marks = [remy_id.as_str(), "alpha", "alpha_mcp", "Alpha key"];
    let b_marks = [bea_id.as_str(), "beta", "beta_mcp", "Beta key"];
    let collections = [
        ("/api/kwants", Some(0)),
        ("/api/channels", None),
        ("/api/teams", Some(1)),
        ("/api/routines", Some(1)),
        ("/api/skills", Some(1)),
        ("/api/mcp/servers", Some(2)),
        ("/api/profiles", Some(3)),
        ("/api/engines", None),
        ("/api/turns", None),
        ("/api/questions", None),
        ("/api/away", None),
    ];
    for (path, own) in collections {
        let in_b = ok(app, "GET", &format!("{path}{q}"), None).await.to_string();
        let in_a = ok(app, "GET", path, None).await.to_string();
        for m in a_marks {
            assert!(!in_b.contains(m), "{path}?space=B shows A's {m}: {in_b}");
        }
        for m in b_marks {
            assert!(!in_a.contains(m), "{path} in Default shows B's {m}: {in_a}");
        }
        if let Some(i) = own {
            assert!(in_b.contains(b_marks[i]), "{path}?space=B misses {}: {in_b}", b_marks[i]);
            assert!(in_a.contains(a_marks[i]), "{path} misses {}: {in_a}", a_marks[i]);
        }
    }
    let channels_b = ok(app, "GET", &format!("/api/channels{q}"), None).await;
    let names: Vec<&str> = channels_b.as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(names, ["general", "team-beta"], "B has only its own channels");
    assert_eq!(channels_b[0]["id"], general_b.as_str());

    // Search: B's message isn't found from A, and A's isn't found from B
    env.state.db.insert_message(&kwnt::db::models::NewMessage {
        channel_id: Some(general_b.clone()),
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: "zebracorn launch plan".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    })
    .await
    .unwrap();
    let found_b = ok(app, "GET", &format!("/api/search?q=zebracorn&space={b}"), None).await.to_string();
    let found_a = ok(app, "GET", "/api/search?q=zebracorn", None).await.to_string();
    assert!(found_b.contains("zebracorn"), "{found_b}");
    assert!(!found_a.contains("zebracorn"), "{found_a}");

    // Unknown and empty space ids are refused, never Default
    for bad in ["/api/kwants?space=spc_nope", "/api/kwants?space=", "/api/events?space=spc_nope"] {
        assert_eq!(call(app, "GET", bad, None).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
    assert_eq!(
        call(app, "POST", "/api/kwants?space=spc_nope", Some(bot("Zed"))).await.0,
        StatusCode::NOT_FOUND
    );

    // SSE ?space=B: only B's events
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(format!("/api/events?space={b}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();
    let send = |e: RuntimeEvent| {
        let _ = env.state.event_sender.send(e);
    };
    send(RuntimeEvent::MemoryChanged { kwant_id: remy_id.clone() });
    send(RuntimeEvent::MemoryChanged { kwant_id: bea_id.clone() });
    let first = next_event(&mut stream).await;
    assert_eq!(first["space_id"], b.as_str(), "A's event was filtered out: {first}");
    // Fail closed: an event with no space never reaches a scoped stream
    send(RuntimeEvent::MemoryChanged { kwant_id: remy_id.clone() });
    send(RuntimeEvent::DiscussionCompleted { total_turns: 1, context_id: None });
    send(RuntimeEvent::MemoryChanged { kwant_id: bea_id.clone() });
    let next = next_event(&mut stream).await;
    assert_eq!(next["space_id"], b.as_str(), "a space-less event leaked: {next}");
    assert_eq!(next["data"]["kwant_id"], bea_id.as_str(), "{next}");

    // The spaces list shows both, Default first
    let list = ok(app, "GET", "/api/spaces", None).await;
    let ids: Vec<&str> = list.as_array().unwrap().iter().filter_map(|s| s["id"].as_str()).collect();
    assert_eq!(ids, ["spc_default", b.as_str()]);
    assert_eq!(list[0]["is_default"], true);
}

#[tokio::test]
async fn archive_pauses_routines_and_restore_brings_back_only_the_active_ones() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Archive me", true).await;
    let q = format!("?space={b}");
    let bea = ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Bea"))).await;
    let bea_id = bea["id"].as_str().unwrap();
    let on = ok(app, "POST", &format!("/api/routines{q}"), Some(routine(bea_id, "on", true))).await;
    let off = ok(app, "POST", &format!("/api/routines{q}"), Some(routine(bea_id, "off", false))).await;
    let enabled = |id: &str| {
        let id = id.to_string();
        let db = env.state.db.clone();
        async move { db.get_routine(&id).await.unwrap().unwrap().enabled }
    };

    let archived = ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": true}))).await;
    assert!(archived["archived_at"].is_string(), "{archived}");
    assert_eq!(archived["paused_routines"], 1);
    assert!(!enabled(on["id"].as_str().unwrap()).await);
    assert!(!enabled(off["id"].as_str().unwrap()).await);

    // Archived: gone from the list, in ?archived=1, and refused under ?space=
    let active = ok(app, "GET", "/api/spaces", None).await.to_string();
    assert!(!active.contains(&b), "{active}");
    let gone = ok(app, "GET", "/api/spaces?archived=1", None).await;
    assert_eq!(gone.as_array().unwrap().len(), 1);
    assert_eq!(gone[0]["id"], b.as_str());
    for (m, path, body) in [
        ("GET", format!("/api/kwants{q}"), None),
        ("POST", format!("/api/kwants{q}"), Some(bot("Late"))),
        ("GET", format!("/api/events{q}"), None),
        ("POST", format!("/api/routines/{}/run", on["id"].as_str().unwrap()), None),
        ("POST", "/api/chat/send".to_string(), Some(json!({"kwant_id": bea_id, "prompt": "hi", "sender_id": "user"}))),
    ] {
        assert_eq!(call(app, m, &path, body).await.0, StatusCode::CONFLICT, "{m} {path}");
    }
    // An item endpoint still reads it
    ok(app, "GET", &format!("/api/spaces/{b}"), None).await;

    let restored = ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": false}))).await;
    assert_eq!(restored["archived_at"], Value::Null);
    assert_eq!(restored["paused_routines"], 0);
    assert!(enabled(on["id"].as_str().unwrap()).await, "the active routine came back");
    assert!(!enabled(off["id"].as_str().unwrap()).await, "the paused-before routine stays off");
    ok(app, "GET", &format!("/api/kwants{q}"), None).await;
}

#[tokio::test]
async fn default_can_be_renamed_but_not_archived_and_names_stay_unique() {
    let env = setup().await;
    let app = &env.app;
    let (s, v) = call(app, "PATCH", "/api/spaces/spc_default", Some(json!({"archived": true}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let renamed = ok(
        app,
        "PATCH",
        "/api/spaces/spc_default",
        Some(json!({"name": "Home", "icon": "🏠", "color": "#ff8800", "sort": 0})),
    )
    .await;
    assert_eq!((renamed["name"].as_str(), renamed["color"].as_str()), (Some("Home"), Some("#ff8800")));
    let cleared = ok(app, "PATCH", "/api/spaces/spc_default", Some(json!({"color": null}))).await;
    assert_eq!(cleared["color"], Value::Null);
    assert_eq!(cleared["name"], "Home", "fields left out stay");

    env.new_space("Work", true).await;
    assert_eq!(
        call(app, "POST", "/api/spaces", Some(json!({"name": "work"}))).await.0,
        StatusCode::CONFLICT
    );
    for bad in [json!({"name": ""}), json!({"name": "x", "bogus": 1}), json!({"name": "a".repeat(61)})] {
        let (s, _) = call(app, "POST", "/api/spaces", Some(bad.clone())).await;
        assert!(s.is_client_error() && s != StatusCode::CONFLICT, "{bad}: {s}");
    }
    assert_eq!(call(app, "PATCH", "/api/spaces/spc_nope", Some(json!({"name": "x"}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(app, "GET", "/api/spaces/spc_nope", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_team_folder_can_not_overlap_another_spaces_folders() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side", true).await;
    let q = format!("?space={b}");
    let remy = ok(app, "POST", "/api/kwants", Some(bot("Remy"))).await;
    let remy_id = remy["id"].as_str().unwrap();
    let bea = ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Bea"))).await;
    let bea_id = bea["id"].as_str().unwrap();
    let b_dir = env.data.join("spaces").join(&b);
    let shared = env._tmp.path().join("shared-repo");

    // A team in A inside B's work folder, and one in B inside A's
    let into_b = b_dir.join("repo").display().to_string();
    let (s, v) = call(app, "POST", "/api/teams", Some(team("a1", remy_id, &[remy_id], Some(&into_b)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Side"), "{v}");
    let into_a = env.data.join("workspace").display().to_string();
    let (s, v) = call(app, "POST", &format!("/api/teams{q}"), Some(team("b1", bea_id, &[bea_id], Some(&into_a)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // A parent of B's folder overlaps too
    let parent = env.data.join("spaces").display().to_string();
    let (s, _) = call(app, "POST", "/api/teams", Some(team("a2", remy_id, &[remy_id], Some(&parent)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Inside its own space's folder is fine
    ok(app, "POST", &format!("/api/teams{q}"), Some(team("b2", bea_id, &[bea_id], Some(&into_b)))).await;

    // An outside folder taken by A's team can't be used by B's, nor by `..` tricks
    let a_team = ok(app, "POST", "/api/teams", Some(team("a3", remy_id, &[remy_id], Some(&shared.display().to_string())))).await;
    let sneaky = shared.join("x").join("..").join("sub").display().to_string();
    let (s, v) = call(app, "POST", &format!("/api/teams{q}"), Some(team("b3", bea_id, &[bea_id], Some(&sneaky)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // Same space may share (S1 behaviour unchanged)
    ok(app, "POST", "/api/teams", Some(team("a4", remy_id, &[remy_id], Some(&sneaky)))).await;

    // Moving a team onto another space's folder is refused as well
    let b_team = ok(app, "POST", &format!("/api/teams{q}"), Some(team("b4", bea_id, &[bea_id], None))).await;
    let mut moved = team("b4", bea_id, &[bea_id], Some(&shared.display().to_string()));
    moved["id"] = b_team["id"].clone();
    let (s, v) = call(app, "PUT", &format!("/api/teams/{}", b_team["id"].as_str().unwrap()), Some(moved)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let _ = a_team;
}

#[tokio::test]
async fn a_general_reply_carries_routing_and_account_and_notifications_name_the_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side", true).await;
    let q = format!("?space={b}");
    let bea = ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Bea"))).await;
    ok(app, "POST", &format!("/api/kwants{q}"), Some(bot("Cy"))).await;
    let general_b = env.space(&b).await["general_channel_id"].as_str().unwrap().to_string();
    let mut events = env.state.event_sender.subscribe();
    let notifier = kwnt::server::continuity::spawn_notifier(env.state.clone());

    ok(
        app,
        "POST",
        "/api/chat/send",
        Some(json!({"channel_id": general_b, "prompt": "Bea, how is the tester work?", "sender_id": "user"})),
    )
    .await;

    let (decided, notify) = tokio::time::timeout(Duration::from_secs(20), async {
        let (mut decided, mut notify) = (None, None);
        while decided.is_none() || notify.is_none() {
            match events.recv().await.map(|e| e.unscoped().clone()) {
                Ok(e @ RuntimeEvent::RoutingDecided { .. }) => decided = Some(e),
                Ok(e @ RuntimeEvent::Notify { .. }) => notify = Some(e),
                _ => {}
            }
        }
        (decided.unwrap(), notify.unwrap())
    })
    .await
    .expect("a routing decision and a notification");
    let RuntimeEvent::RoutingDecided { kwant_id, candidate_count, fallback, matched_terms, .. } = decided else {
        unreachable!()
    };
    assert_eq!(kwant_id, bea["id"].as_str().unwrap(), "keywords pick Bea by name");
    assert_eq!(candidate_count, Some(2));
    assert!(fallback, "Laya is offline in tests");
    assert!(matched_terms.iter().any(|t| t == "bea"), "{matched_terms:?}");
    let RuntimeEvent::Notify { space_id, space_name, title, .. } = notify else { unreachable!() };
    assert_eq!(space_id.as_deref(), Some(b.as_str()));
    assert_eq!(space_name.as_deref(), Some("Side"));
    assert!(title.starts_with("Side · "), "two spaces, so the title names it: {title}");

    let msgs = env.state.db.list_messages(&general_b, 50).await.unwrap();
    let reply = msgs.iter().find(|m| m.sender_role == "kwant").expect("Bea's reply");
    let meta: Value = serde_json::from_str(reply.meta.as_deref().expect("reply meta")).unwrap();
    assert_eq!(meta["routing"]["kwant_id"], bea["id"]);
    assert_eq!(meta["routing"]["candidate_count"], 2);
    assert_eq!(meta["routing"]["fallback"], true);
    assert_eq!(meta["ran_on"]["account"], "This machine", "{meta}");
    notifier.abort();
}
