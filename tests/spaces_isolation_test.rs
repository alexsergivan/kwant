//! Spaces S1: nothing in one space reaches another. Each test makes a second space B straight in
//! the db next to the Default space A, with the same names in both where that is the risk (a bot
//! "Ada", a team "web"), and checks one path that used to look across every space. A fake claude
//! CLI on PATH records the folder it ran in and the prompt, so no real engine is ever called.
//! Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use futures::StreamExt;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, ProfileKey, NewChannel, NewKwant, NewLlmProfile, NewMessage, NewTeam},
        spaces::{NewSpace, SpaceId},
        Database,
    },
    profiles::Engine,
    router::LayaRouter,
    server::{continuity::SelfScope, continuity::TurnOrigin, create_router, delegation, profile_turn, AppState},
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// Answers "answer <tag>" for a prompt with "REPLYTAG:<tag>", or "answer @Ada" for one with
/// "SAYADA". Every call is saved under `$KWNT_TEST_PROMPTS` as its folder, a "---" line and the prompt.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s\n---\n%s' "$PWD" "$last" > "$KWNT_TEST_PROMPTS/$$-$(date +%s%N)"
tag=$(printf '%s\n' "$last" | sed -n 's/.*REPLYTAG:\([A-Za-z0-9_]*\).*/\1/p' | tail -n 1)
reply="answer ${tag:-none}"
case "$last" in *SAYADA*) reply="answer @Ada" ;; esac
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"$reply\"}"
"#;

fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        prompts
    })
}

/// (folder, prompt) of every fake CLI call whose prompt contains `marker`.
fn calls_with(marker: &str) -> Vec<(String, String)> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| p.contains(marker))
        .map(|p| {
            let (dir, prompt) = p.split_once("\n---\n").unwrap_or(("", &p));
            (dir.to_string(), prompt.to_string())
        })
        .collect()
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

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The Default space A and a second space B, each with a bot "Ada" and a second bot, a #general
/// and a team "web" (Ada leads, both are members).
struct Env {
    app: Router,
    state: AppState,
    data: PathBuf,
    a: SpaceId,
    b: SpaceId,
    ada_a: Kwant,
    ada_b: Kwant,
    other_b: Kwant,
    _tmp: tempfile::TempDir,
}

async fn kwant(db: &Database, space: &SpaceId, name: &str) -> Kwant {
    db.create_kwant(
        space,
        &NewKwant {
            name: name.into(),
            title: format!("{name} the tester"),
            persona_prompt: "Test persona".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "api".into(),
            preferred_model: "claude".into(),
        },
    )
    .await
    .unwrap()
}

async fn setup(b_this_machine_on: bool) -> Env {
    prompts_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let a = SpaceId::default_space();
    let b = SpaceId::new(
        &db.create_space(&NewSpace {
            name: "Side project".into(),
            icon: "🧪".into(),
            color: None,
            this_machine_enabled: b_this_machine_on,
        })
        .await
        .unwrap()
        .id,
    );
    let ada_a = kwant(&db, &a, "Ada").await;
    let other_a = kwant(&db, &a, "Remy").await;
    let ada_b = kwant(&db, &b, "Ada").await;
    let other_b = kwant(&db, &b, "Bea").await;
    for (space, ada, other) in [(&a, &ada_a, &other_a), (&b, &ada_b, &other_b)] {
        let members = vec![ada.id.clone(), other.id.clone()];
        if space != &a {
            // Default got its #general from the seed
            db.create_channel(space, &NewChannel {
                name: "general".into(),
                description: None,
                lead_kwant_id: ada.id.clone(),
                member_kwants: members.clone(),
                routing_mode: "auto".into(),
            })
            .await
            .unwrap();
        }
        db.create_team(space, &NewTeam {
            name: "web".into(),
            description: None,
            icon: None,
            lead_kwant_id: ada.id.clone(),
            member_kwants: members,
            workspace_path: None,
            repo_url: None,
        })
        .await
        .unwrap();
    }
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
    Env { app, state, data, a, b, ada_a, ada_b, other_b, _tmp: tmp }
}

impl Env {
    async fn general(&self, space: &SpaceId) -> String {
        self.state.db.general_channel(space).await.unwrap().expect("a #general").id
    }

    async fn team(&self, space: &SpaceId) -> String {
        self.state.db.get_team_by_name(space, "web").await.unwrap().expect("team web").id
    }

    async fn send(&self, body: Value) {
        let (s, v) = call(&self.app, "POST", "/api/chat/send", Some(body)).await;
        assert!(s.is_success(), "{s} {v}");
    }

    /// Messages in a channel or direct chat.
    async fn messages(&self, chat: &str) -> Vec<kwnt::db::models::Message> {
        self.state.db.list_messages(chat, 200).await.unwrap()
    }

    async fn say(&self, chat_channel: Option<&str>, direct: Option<&str>, sender: &str, text: &str) {
        self.state
            .db
            .insert_message(&NewMessage {
                channel_id: chat_channel.map(str::to_string),
                direct_kwant_id: direct.map(str::to_string),
                sender_role: "user".into(),
                sender_id: sender.into(),
                text: text.into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 1,
            })
            .await
            .unwrap();
    }
}

// 1. Laya's #general candidates

#[tokio::test]
async fn general_routing_candidates_come_only_from_the_channels_space() {
    let env = setup(true).await;
    let mut events = env.state.event_sender.subscribe();
    let general_b = env.general(&env.b).await;
    env.send(json!({"channel_id": general_b, "prompt": "who is around? REPLYTAG:laya1", "sender_id": "user"})).await;

    let candidates = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(RuntimeEvent::RoutingStarted { candidates, .. }) = events.recv().await.map(|e| e.unscoped().clone()) {
                return candidates;
            }
        }
    })
    .await
    .expect("routing started");
    let ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "only B's two bots: {ids:?}");
    assert!(ids.contains(&env.ada_b.id.as_str()) && ids.contains(&env.other_b.id.as_str()));
    assert!(!ids.contains(&env.ada_a.id.as_str()));
}

// 2. @mentions

#[tokio::test]
async fn a_mention_reaches_the_bot_of_that_name_in_the_chats_space() {
    let env = setup(true).await;
    let mut events = env.state.event_sender.subscribe();
    // In B's #general, "@Ada" is B's Ada even though A has a bot of the same name
    let general_b = env.general(&env.b).await;
    env.send(json!({"channel_id": general_b, "prompt": "@Ada hello REPLYTAG:mention1", "sender_id": "user"})).await;
    wait_for("Ada's reply in B", || async {
        env.messages(&general_b).await.iter().any(|m| m.text.contains("answer mention1"))
    })
    .await;
    // Replies are saved under the bot's name, so tell the two Adas apart by the turns' kwant ids
    let speakers = turn_kwants(&mut events);
    assert!(speakers.contains(&env.ada_b.id), "{speakers:?}");
    assert!(!speakers.contains(&env.ada_a.id), "{speakers:?}");
    assert!(env.messages(&env.ada_a.id).await.is_empty());
}

/// Kwant ids of the turns in the events received so far.
fn turn_kwants(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        let e = match events.try_recv() {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        };
        if let RuntimeEvent::Scoped { tag, .. } = &e {
            ids.push(tag.kwant_id.clone());
        }
        if let RuntimeEvent::TurnComplete { kwant_id: Some(k), .. } = e.unscoped() {
            ids.push(k.clone());
        }
    }
    ids
}

// 3. Delegation without a member list

#[tokio::test]
async fn delegation_without_a_member_list_stays_in_the_leads_space() {
    let env = setup(true).await;
    // The agent tool in a direct chat, which has no member list: A's bots aren't offered or reachable
    let scope = SelfScope {
        kwant_id: env.other_b.id.clone(),
        kwant_name: env.other_b.name.clone(),
        origin: Some(TurnOrigin {
            channel_id: None,
            direct_kwant_id: Some(env.other_b.id.clone()),
            thread_id: None,
            team_id: None,
            member_ids: Vec::new(),
            depth: 0,
        }),
        cancel: None,
        turn_id: None,
        workspace: None,
    };
    let (text, is_error) = delegation::tool_delegate(&env.state, &scope, &json!({"to": "Remy", "task": "help"})).await;
    assert!(is_error, "{text}");
    assert!(text.contains("No agent named 'Remy'"), "{text}");
    assert!(text.contains("Ada") && !text.contains("Remy."), "only B's bots are offered: {text}");

    // The HTTP API, which takes ids: a specialist from another space is refused
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({"lead_kwant_id": env.other_b.id, "specialist_kwant_id": env.ada_a.id, "task": "help", "kwant_id": env.other_b.id})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(env.state.db.list_delegations(None, None, 50).await.unwrap().is_empty());
}

// 4. Account failover and default selection

#[tokio::test]
async fn failover_only_tries_accounts_in_the_kwants_space() {
    let env = setup(true).await;
    // A has a Claude API key account first in line; B has only its own This machine
    let in_a = env
        .state
        .db
        .create_profile(&env.a, &NewLlmProfile {
            engine: "claude".into(),
            name: "Work key".into(),
            kind: "api_key".into(),
            home_dir: None,
            priority: 1,
            enabled: true,
            is_default: true,
        })
        .await
        .unwrap();
    env.state.db.set_profile_key(&in_a.id, &ProfileKey("sk-ant-test-a".into())).await.unwrap();
    let plan_a = profile_turn::plan(&env.state, Some(Engine::Claude), &env.ada_a.id).await;
    assert!(plan_a.iter().any(|p| p.id == in_a.id), "A uses its own account");
    let plan_b = profile_turn::plan(&env.state, Some(Engine::Claude), &env.ada_b.id).await;
    assert!(!plan_b.is_empty(), "B has its own This machine");
    assert!(plan_b.iter().all(|p| p.space_id == env.b.as_str()), "{:?}", plan_b.iter().map(|p| &p.id).collect::<Vec<_>>());
    // Binding B's bot to A's account is refused
    assert!(matches!(
        env.state.db.set_kwant_binding(&env.ada_b.id, Some(Some(&in_a.id)), None).await,
        Err(kwnt::db::DbError::OtherSpace(_))
    ));
}

// 5. This machine off is never a fallback

#[tokio::test]
async fn this_machine_off_in_a_space_is_never_used_even_with_accounts_elsewhere() {
    let env = setup(false).await;
    let in_a = env
        .state
        .db
        .create_profile(&env.a, &NewLlmProfile {
            engine: "claude".into(),
            name: "Work key".into(),
            kind: "api_key".into(),
            home_dir: None,
            priority: 1,
            enabled: true,
            is_default: false,
        })
        .await
        .unwrap();
    // A ready account in A, which B must not borrow
    env.state.db.set_profile_key(&in_a.id, &ProfileKey("sk-ant-test-a".into())).await.unwrap();
    assert!(profile_turn::host_off(&env.state, &env.b).await.contains(&Engine::Claude));
    assert!(!profile_turn::host_off(&env.state, &env.a).await.contains(&Engine::Claude));
    assert!(profile_turn::plan(&env.state, Some(Engine::Claude), &env.ada_b.id).await.is_empty());

    env.send(json!({"kwant_id": env.ada_b.id, "prompt": "hi REPLYTAG:hostoff1", "sender_id": "user"})).await;
    for _ in 0..100 {
        if env.messages(&env.ada_b.id).await.iter().any(|m| m.text.contains("This machine is off in this space")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let texts: Vec<String> = env.messages(&env.ada_b.id).await.into_iter().map(|m| m.text).collect();
    assert!(texts.iter().any(|t| t.contains("This machine is off in this space")), "{texts:?}");
    assert!(calls_with("hostoff1").is_empty(), "the CLI on this machine was not run");
    assert!(!env.messages(&env.ada_b.id).await.iter().any(|m| m.text.contains("answer hostoff1")));
}

// 6. MCP servers

#[tokio::test]
async fn mcp_servers_never_reach_bots_in_another_space() {
    let env = setup(true).await;
    // Created through the API, which is the Default space until S2
    let (s, server) = call(
        &env.app,
        "POST",
        "/api/mcp/servers",
        Some(json!({"name": "files", "transport": "http", "url": "http://127.0.0.1:9/mcp"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{server}");
    let id = server["id"].as_str().unwrap().to_string();
    let ids = |list: Vec<kwnt::db::models::McpServer>| list.into_iter().map(|s| s.id).collect::<Vec<_>>();

    assert!(ids(env.state.db.list_active_mcp_servers_for_kwant(&env.ada_a.id).await.unwrap()).contains(&id), "on by default in A");
    assert!(ids(env.state.db.list_active_mcp_servers_for_kwant(&env.ada_b.id).await.unwrap()).is_empty());
    assert!(env.state.db.list_mcp_servers(&env.b).await.unwrap().is_empty());

    let (s, v) = call(&env.app, "PUT", &format!("/api/kwants/{}/mcp-servers", env.ada_b.id), Some(json!({"server_ids": [id]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // Even written straight to the db, the assignment doesn't cross
    let _ = env.state.db.set_kwant_mcp_servers(&env.ada_b.id, std::slice::from_ref(&id)).await;
    assert!(ids(env.state.db.list_active_mcp_servers_for_kwant(&env.ada_b.id).await.unwrap()).is_empty());
    assert!(env.state.db.get_kwant_mcp_server_ids(&env.ada_b.id).await.unwrap().is_empty());
}

// 7. Skills

#[tokio::test]
async fn shared_skills_stay_in_their_space() {
    let env = setup(true).await;
    let (s, skill) = call(
        &env.app,
        "POST",
        "/api/skills",
        Some(json!({"name": "release-notes", "description": "Write release notes", "body": "Steps for release notes."})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{skill}");
    let id = skill["id"].as_str().unwrap();
    let names = |list: Vec<kwnt::db::skills::Skill>| list.into_iter().map(|s| s.id).collect::<Vec<_>>();
    assert!(names(env.state.db.enabled_skills_for(&env.ada_a.id).await.unwrap()).contains(&id.to_string()));
    assert!(env.state.db.enabled_skills_for(&env.ada_b.id).await.unwrap().is_empty());
    let (_, list) = call(&env.app, "GET", &format!("/api/skills?kwant_id={}", env.ada_b.id), None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(0), "{list}");
    assert!(env.state.db.list_skills(&env.b, None, None).await.unwrap().is_empty());

    // A skill for B's bot lives in B, and A's list doesn't show it
    let (s, own) = call(
        &env.app,
        "POST",
        "/api/skills",
        Some(json!({"name": "triage", "description": "Triage bugs", "body": "Steps.", "kwant_id": env.ada_b.id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{own}");
    assert_eq!(own["space_id"], env.b.as_str());
    let (_, list_a) = call(&env.app, "GET", "/api/skills", None).await;
    assert!(!list_a.to_string().contains("triage"));
}

// 8. The search tool

#[tokio::test]
async fn the_search_tool_only_finds_messages_in_the_agents_space() {
    let env = setup(true).await;
    let general_a = env.general(&env.a).await;
    let general_b = env.general(&env.b).await;
    env.say(Some(&general_a), None, "user", "the zebra launch plan for A").await;
    env.say(None, Some(&env.ada_a.id), "user", "secret zebra numbers in A's direct chat").await;
    env.say(Some(&general_b), None, "user", "a zebra sketch for B").await;

    let token = "tok-search-b";
    env.state.self_tools.lock().await.insert(
        token.into(),
        SelfScope {
            kwant_id: env.ada_b.id.clone(),
            kwant_name: env.ada_b.name.clone(),
            origin: None,
            cancel: None,
            turn_id: None,
            workspace: None,
        },
    );
    let resp = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp/self")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search_history", "arguments": {"query": "zebra"}}})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string();
    assert!(body.contains("sketch for B"), "B's own message is found: {body}");
    assert!(!body.contains("launch plan") && !body.contains("secret zebra"), "{body}");

    // The HTTP search, scoped by the bot it names
    let (_, hits) = call(&env.app, "GET", &format!("/api/search?q=zebra&kwant_id={}", env.ada_b.id), None).await;
    assert!(!hits.to_string().contains("launch plan") && !hits.to_string().contains("secret zebra"), "{hits}");
    let all_b = env.state.db.search_messages(&env.b, "zebra", &kwnt::db::continuity::SearchFilter {
        channel_id: None,
        kwant_id: None,
        allowed_channels: None,
        allowed_direct: None,
        limit: 50,
    })
    .await
    .unwrap();
    assert_eq!(all_b.len(), 1);
}

// 9. Routines and the scheduler

#[tokio::test]
async fn a_routine_cannot_join_a_bot_and_a_channel_from_different_spaces() {
    let env = setup(true).await;
    let general_a = env.general(&env.a).await;
    let schedule = json!({"type": "interval", "every_minutes": 60});
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.ada_b.id, "channel_id": general_a, "name": "Digest", "instructions": "Summarise", "schedule": schedule})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // A routine in B's own direct chat works, and lands in B, not in A's list
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.ada_b.id, "channel_id": null, "name": "Digest", "instructions": "Summarise", "schedule": schedule})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let (_, list_a) = call(&env.app, "GET", "/api/routines", None).await;
    assert_eq!(list_a.as_array().map(Vec::len), Some(0), "{list_a}");
    let in_b = env.state.db.list_routines(&env.b).await.unwrap();
    assert_eq!(in_b.len(), 1);
    assert_eq!(in_b[0].space_id, env.b.as_str());
    assert!(env.state.db.list_routines(&env.a).await.unwrap().is_empty());

    // Moving it onto A's channel is refused as well
    let id = r["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "PATCH", &format!("/api/routines/{id}"), Some(json!({"channel_id": general_a}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

// 10. Work folders

#[tokio::test]
async fn teams_with_the_same_name_work_in_different_folders() {
    let env = setup(true).await;
    let team_a = env.team(&env.a).await;
    let team_b = env.team(&env.b).await;
    env.send(json!({"team_id": team_a, "prompt": "@Ada build it REPLYTAG:folderA", "sender_id": "user"})).await;
    env.send(json!({"team_id": team_b, "prompt": "@Ada build it REPLYTAG:folderB", "sender_id": "user"})).await;
    wait_for("both teams' turns", || async { !calls_with("folderA").is_empty() && !calls_with("folderB").is_empty() }).await;

    let dir_a = PathBuf::from(&calls_with("folderA")[0].0);
    let dir_b = PathBuf::from(&calls_with("folderB")[0].0);
    assert_ne!(dir_a, dir_b);
    let space_root = env.data.join("spaces");
    assert!(!dir_a.starts_with(&space_root), "A keeps its folder: {}", dir_a.display());
    assert!(dir_b.starts_with(&space_root), "B works under its own space folder: {}", dir_b.display());
    assert!(!dir_b.starts_with(&dir_a) && !dir_a.starts_with(&dir_b));
}

// 11. SSE events carry their space

/// The next SSE event's JSON.
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
async fn sse_events_carry_the_space_they_belong_to() {
    let env = setup(true).await;
    let resp = env.app.clone().oneshot(Request::builder().uri("/api/events").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();
    let general_b = env.general(&env.b).await;
    let send = |e: RuntimeEvent| {
        let _ = env.state.event_sender.send(e);
    };
    send(RuntimeEvent::MemoryChanged { kwant_id: env.ada_b.id.clone() });
    assert_eq!(next_event(&mut stream).await["space_id"], env.b.as_str());
    send(RuntimeEvent::MemoryChanged { kwant_id: env.ada_a.id.clone() });
    assert_eq!(next_event(&mut stream).await["space_id"], env.a.as_str());
    send(RuntimeEvent::RoutingStarted { context_id: Some(general_b), candidates: Vec::new() });
    assert_eq!(next_event(&mut stream).await["space_id"], env.b.as_str());
    send(RuntimeEvent::DiscussionCompleted { total_turns: 1 });
    assert_eq!(next_event(&mut stream).await["space_id"], Value::Null, "an event with no chat is global");
}

// Handoffs (bonus)

#[tokio::test]
async fn a_handoff_mention_never_reaches_a_bot_in_another_space() {
    let env = setup(true).await;
    let mut events = env.state.event_sender.subscribe();
    // Bea in B answers "@Ada": both spaces have an Ada, and only B's may take the handoff
    let general_b = env.general(&env.b).await;
    env.send(json!({"channel_id": general_b, "prompt": "@Bea REPLYTAG:handoff1 SAYADA", "sender_id": "user"})).await;
    wait_for("Bea's reply", || async {
        env.messages(&general_b).await.iter().any(|m| m.text.contains("answer @Ada"))
    })
    .await;
    // Bea's "@Ada" hands off to B's Ada, never to A's
    wait_for("the handoff turn", || async {
        calls_with("handed this conversation to you").iter().any(|(_, p)| p.contains("Bea handed"))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a_msgs = env.messages(&env.general(&env.a).await).await;
    assert!(!a_msgs.iter().any(|m| m.sender_id == env.ada_a.id), "A's Ada never spoke");
    let handoff_replies: Vec<_> = env.messages(&general_b).await.into_iter().filter(|m| m.sender_id == env.ada_a.id).collect();
    assert!(handoff_replies.is_empty());
    let speakers = turn_kwants(&mut events);
    assert!(speakers.contains(&env.ada_b.id), "B's Ada took the handoff: {speakers:?}");
    assert!(!speakers.contains(&env.ada_a.id), "{speakers:?}");
}

// Startup and #general

#[tokio::test]
async fn a_second_general_in_a_space_is_refused_but_each_space_has_its_own() {
    let env = setup(true).await;
    let again = env
        .state
        .db
        .create_channel(&env.b, &NewChannel {
            name: "general".into(),
            description: None,
            lead_kwant_id: env.ada_b.id.clone(),
            member_kwants: vec![env.ada_b.id.clone()],
            routing_mode: "auto".into(),
        })
        .await;
    assert!(matches!(again, Err(kwnt::db::DbError::Conflict(_))));
    assert_ne!(env.general(&env.a).await, env.general(&env.b).await);
    // A channel can't take members from another space
    let mixed = env
        .state
        .db
        .create_channel(&env.b, &NewChannel {
            name: "mixed".into(),
            description: None,
            lead_kwant_id: env.ada_b.id.clone(),
            member_kwants: vec![env.ada_b.id.clone(), env.ada_a.id.clone()],
            routing_mode: "auto".into(),
        })
        .await;
    assert!(matches!(mixed, Err(kwnt::db::DbError::OtherSpace(_))));
}

// Routines edited by hand across spaces

/// Inserts a routine straight into the db, as the API refuses to make one across spaces, and
/// runs a scheduler tick. Returns its runs once the run has finished.
async fn tick_routine(env: &Env, id: &str, kwant_id: &str, channel_id: Option<&str>, space: &SpaceId, tag: &str) -> Vec<kwnt::db::continuity::RoutineRun> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!("sqlite://{}", env.data.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO routines (id, kwant_id, channel_id, name, instructions, schedule, enabled, next_run_at, created_at, space_id) \
         VALUES (?, ?, ?, 'Evil', ?, '{\"type\":\"interval\",\"every_minutes\":60}', 1, '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z', ?)",
    )
    .bind(id)
    .bind(kwant_id)
    .bind(channel_id)
    .bind(format!("REPLYTAG:{tag}"))
    .bind(space.as_str())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(kwnt::server::scheduler::tick(&env.state, chrono::Utc::now()).await, 1);
    let mut runs = Vec::new();
    wait_for("the run to finish", || {
        let db = env.state.db.clone();
        let id = id.to_string();
        async move {
            let r = db.list_routine_runs(&id, 10).await.unwrap();
            let done = r.iter().all(|r| r.status != "running") && !r.is_empty();
            done
        }
    })
    .await;
    runs.extend(env.state.db.list_routine_runs(id, 10).await.unwrap());
    runs
}

#[tokio::test]
async fn the_scheduler_does_not_run_a_routine_whose_bot_and_channel_are_in_different_spaces() {
    let env = setup(true).await;
    let general_a = env.general(&env.a).await;
    let before = env.messages(&general_a).await.len();
    let runs = tick_routine(&env, "rx_channel", &env.ada_b.id, Some(&general_a), &env.a, "xspace_channel").await;

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "error");
    assert!(runs[0].error.as_deref().unwrap_or("").contains("another space"), "{:?}", runs[0].error);
    assert!(calls_with("xspace_channel").is_empty(), "B's bot never ran");
    assert_eq!(env.messages(&general_a).await.len(), before, "nothing was posted in A's #general");
    assert!(env.messages(&env.ada_b.id).await.is_empty());
}

#[tokio::test]
async fn the_scheduler_does_not_run_a_direct_chat_routine_for_a_bot_in_another_space() {
    let env = setup(true).await;
    // A direct-chat routine filed under A whose bot is B's Ada
    let runs = tick_routine(&env, "rx_direct", &env.ada_b.id, None, &env.a, "xspace_direct").await;

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "error");
    assert!(runs[0].error.as_deref().unwrap_or("").contains("another space"), "{:?}", runs[0].error);
    assert!(calls_with("xspace_direct").is_empty(), "B's bot never ran");
    assert!(env.messages(&env.ada_b.id).await.is_empty(), "nothing was posted in B's direct chat");
    assert!(env.messages(&env.ada_a.id).await.is_empty());
}
