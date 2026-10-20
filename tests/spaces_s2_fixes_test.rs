//! Spaces S2 QA fixes: the `?space=` SSE stream with real events from both spaces, team names that
//! try to leave their space's folder, an explicit `?space=` on item routes that must match the row,
//! skill owners and chat cancel scoped to the space, and the archive gaps (question answers, the
//! scheduler, writes on archived rows, and a live CLI child killed with its whole process group).
//! A fake claude CLI on PATH answers turns; `SLEEPY:<tag>` in the prompt makes it start a `sleep`
//! child, write both pids under `KWNT_S2_PIDS`, and wait. Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use futures::StreamExt;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{questions::QuestionPlace, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s' "$last" > "$KWNT_S2_PIDS/inv-$$"
tag=$(printf '%s' "$last" | sed -n 's/.*SLEEPY:\([A-Za-z0-9_]*\).*/\1/p' | tail -n 1)
if [ -n "$tag" ]; then
  echo $$ > "$KWNT_S2_PIDS/$tag.cli"
  sleep 60 &
  echo $! > "$KWNT_S2_PIDS/$tag.child"
  wait
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer ok"}'
"#;

fn pids_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let pids = root.join("pids");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&pids).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_S2_PIDS", &pids);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        pids
    })
}

/// How many CLI turns so far had `marker` in their prompt.
fn invocations_with(marker: &str) -> usize {
    std::fs::read_dir(pids_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("inv-"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter(|p| p.contains(marker))
        .count()
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

async fn call_as(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
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
    pids_dir();
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

impl Env {
    async fn new_space(&self, name: &str) -> String {
        let (s, v) = call(&self.app, "POST", "/api/spaces", Some(json!({"name": name, "this_machine_enabled": true}))).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    async fn bot(&self, space: &str, name: &str) -> String {
        let v = ok(&self.app, "POST", &format!("/api/kwants?space={space}"), Some(bot(name))).await;
        v["id"].as_str().unwrap().to_string()
    }

    async fn general(&self, space: &str) -> String {
        let v = ok(&self.app, "GET", &format!("/api/spaces/{space}"), None).await;
        v["general_channel_id"].as_str().expect("#general").to_string()
    }

    async fn question(&self, kwant: &str) -> String {
        let place = QuestionPlace {
            kwant_id: kwant.into(),
            channel_id: None,
            direct_kwant_id: Some(kwant.into()),
            thread_id: None,
            turn_id: None,
        };
        let q = self.state.db.create_question(&place, "Which one?", &["left".into(), "right".into()]).await.unwrap();
        q.id
    }

    async fn idle(&self) {
        let mut quiet = 0;
        for _ in 0..300 {
            quiet = if self.state.turns.active_turns().is_empty() { quiet + 1 } else { 0 };
            if quiet >= 10 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("turns never finished: {:?}", self.state.turns.active_turns());
    }
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

fn team(name: &str, lead: &str, workspace_path: Option<&str>) -> Value {
    json!({"name": name, "lead_kwant_id": lead, "member_kwants": [lead], "workspace_path": workspace_path})
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

fn skill(name: &str, kwant_id: Option<&str>) -> Value {
    json!({"name": name, "description": "Notes", "body": "Steps.", "kwant_id": kwant_id})
}

fn mcp(name: &str) -> Value {
    json!({"name": name, "transport": "http", "url": "https://example.com/mcp"})
}

fn profile(name: &str) -> Value {
    json!({"engine": "claude", "name": name, "kind": "api_key"})
}

/// Collects every event of an SSE stream in the background until `stop` is dropped or sent.
async fn record(app: &Router, uri: String) -> (tokio::task::JoinHandle<Vec<Value>>, tokio::sync::oneshot::Sender<()>) {
    let app = app.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let resp = app.oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let _ = ready_tx.send(());
        let mut stream = resp.into_body().into_data_stream();
        let mut events = Vec::new();
        let mut buf = String::new();
        loop {
            let chunk = tokio::select! {
                _ = &mut stop_rx => break,
                c = stream.next() => match c {
                    Some(Ok(c)) => c,
                    _ => break,
                },
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buf.find("\n\n") {
                let frame: String = buf.drain(..end + 2).collect();
                for line in frame.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                            events.push(v);
                        }
                    }
                }
            }
        }
        events
    });
    // The subscription is made when the response is built
    let _ = ready_rx.await;
    (handle, stop_tx)
}

/// Is the process gone (or only a zombie waiting for its parent)?
fn gone(pid: &str) -> bool {
    match std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
        Err(_) => true,
        Ok(stat) => stat.rsplit(')').next().map(|r| r.trim_start().starts_with('Z')).unwrap_or(true),
    }
}

async fn wait_file(path: &Path) -> String {
    for _ in 0..200 {
        if let Ok(s) = std::fs::read_to_string(path) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{} never appeared", path.display());
}

async fn wait_gone(pid: &str, what: &str) {
    for _ in 0..100 {
        if gone(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} (pid {pid}) is still running");
}

/// Starts a direct turn for `kwant` that sleeps in the fake CLI; returns (cli pid, child pid).
async fn sleepy_turn(env: &Env, kwant: &str, tag: &str) -> (String, String) {
    ok(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kwant, "prompt": format!("Work slowly SLEEPY:{tag}"), "sender_id": "user"})),
    )
    .await;
    let dir = pids_dir();
    (wait_file(&dir.join(format!("{tag}.cli"))).await, wait_file(&dir.join(format!("{tag}.child"))).await)
}

#[tokio::test]
async fn scoped_event_streams_see_only_their_own_space_for_real_activity() {
    let env = setup().await;
    let app = &env.app;
    let a = "spc_default".to_string();
    let b = env.new_space("Side").await;
    let remy = env.bot(&a, "Remy").await;
    let rae = env.bot(&a, "Rae").await;
    let bea = env.bot(&b, "Bea").await;
    let cy = env.bot(&b, "Cy").await;

    let (in_a, stop_a) = record(app, format!("/api/events?space={a}")).await;
    let (in_b, stop_b) = record(app, format!("/api/events?space={b}")).await;
    let (all, stop_all) = record(app, "/api/events".into()).await;

    for (space, lead, specialist, name) in [(&a, &remy, &rae, "Remy"), (&b, &bea, &cy, "Bea")] {
        let general = env.general(space).await;
        // A #general message
        ok(
            app,
            "POST",
            "/api/chat/send",
            Some(json!({"channel_id": general, "prompt": format!("{name}, how is it going?"), "sender_id": "user"})),
        )
        .await;
        // A thread
        ok(app, "POST", &format!("/api/kwants/{lead}/threads"), Some(json!({"title": "A side thread"}))).await;
        // A delegation
        let (s, v) = call(
            app,
            "POST",
            "/api/delegations",
            Some(json!({"lead_kwant_id": lead, "specialist_kwant_id": specialist, "task": "Check it", "kwant_id": lead, "continue_lead": false})),
        )
        .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        // A routine run
        let r = ok(app, "POST", &format!("/api/routines?space={space}"), Some(routine(lead, "digest", false))).await;
        ok(app, "POST", &format!("/api/routines/{}/run", r["id"].as_str().unwrap()), None).await;
        // A question answer
        let q = env.question(lead).await;
        ok(app, "POST", &format!("/api/questions/{q}/answer"), Some(json!({"choice": 0}))).await;
    }
    env.idle().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    for stop in [stop_a, stop_b, stop_all] {
        let _ = stop.send(());
    }

    let (in_a, in_b, all) = (in_a.await.unwrap(), in_b.await.unwrap(), all.await.unwrap());
    let kinds = |events: &[Value]| events.iter().filter_map(|e| e["type"].as_str().map(String::from)).collect::<BTreeSet<_>>();
    for (space, events) in [(&a, &in_a), (&b, &in_b)] {
        for e in events {
            assert_eq!(e["space_id"].as_str(), Some(space.as_str()), "the {space} stream got another space's event: {e}");
        }
        let seen = kinds(events);
        for want in ["message_added", "thread_updated", "delegation_updated", "routine_run", "question_updated"] {
            assert!(seen.contains(want), "the {space} stream never saw its own {want}: {seen:?}");
        }
    }
    // The unscoped stream is unchanged: it sees both spaces
    let spaces: BTreeSet<_> = all.iter().filter_map(|e| e["space_id"].as_str()).collect();
    assert!(spaces.contains(a.as_str()) && spaces.contains(b.as_str()), "{spaces:?}");
}

#[tokio::test]
async fn team_names_can_not_leave_the_space_folder_and_collisions_ignore_case() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let remy = env.bot("spc_default", "Remy").await;
    let bea = env.bot(&b, "Bea").await;

    let escapes = [
        ("spc_default", remy.as_str(), format!("../spaces/{b}/pwnD")),
        (b.as_str(), bea.as_str(), "../../../workspaces/pwnB".to_string()),
    ];
    for (space, lead, name) in &escapes {
        let (s, v) = call(app, "POST", &format!("/api/teams?space={space}"), Some(team(name, lead, None))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name}: {v}");
    }
    assert!(!env.data.join("spaces").join(&b).join("pwnD").exists());
    assert!(!env.data.join("workspaces").join("pwnB").exists());
    for name in ["a/b", "a\\b", "..", "x..y", ".hidden", "nul\0x"] {
        let (s, v) = call(app, "POST", "/api/teams", Some(team(name, &remy, None))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name:?}: {v}");
    }

    // Renaming onto an escape is refused too
    let t = ok(app, "POST", &format!("/api/teams?space={b}"), Some(team("beta", &bea, None))).await;
    let tid = t["id"].as_str().unwrap();
    let mut renamed = team("../../../workspaces/pwnB", &bea, None);
    renamed["id"] = t["id"].clone();
    let (s, v) = call(app, "PUT", &format!("/api/teams/{tid}"), Some(renamed)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // An outside folder taken in one space is taken in any letter case
    let shared = env._tmp.path().join("Shared-Repo");
    ok(app, "POST", "/api/teams", Some(team("alpha", &remy, Some(&shared.display().to_string())))).await;
    let lower = env._tmp.path().join("shared-repo").join("sub").display().to_string();
    let (s, v) = call(app, "POST", &format!("/api/teams?space={b}"), Some(team("beta2", &bea, Some(&lower)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn an_explicit_space_on_an_item_route_must_be_the_rows_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    env.bot(&b, "Bea").await;
    let remy = env.bot("spc_default", "Remy").await;
    let general = env.general("spc_default").await;
    let team_id = ok(app, "POST", "/api/teams", Some(team("alpha", &remy, None))).await["id"].as_str().unwrap().to_string();
    let routine_id = ok(app, "POST", "/api/routines", Some(routine(&remy, "digest", false))).await["id"].as_str().unwrap().to_string();
    let skill_id = ok(app, "POST", "/api/skills", Some(skill("alpha-notes", None))).await["id"].as_str().unwrap().to_string();
    let mcp_id = ok(app, "POST", "/api/mcp/servers", Some(mcp("alpha_mcp"))).await["id"].as_str().unwrap().to_string();
    let profile_id = ok(app, "POST", "/api/profiles", Some(profile("Alpha key"))).await["id"].as_str().unwrap().to_string();
    let thread_id = ok(app, "POST", &format!("/api/kwants/{remy}/threads"), Some(json!({"title": "t"}))).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let question_id = env.question(&remy).await;

    let reads = [
        format!("/api/channels/{general}"),
        format!("/api/channels/{general}/messages"),
        format!("/api/kwants/{remy}/memories"),
        format!("/api/kwants/{remy}/threads"),
        format!("/api/kwants/{remy}/mcp-servers"),
        format!("/api/teams/{team_id}"),
        format!("/api/routines/{routine_id}"),
        format!("/api/routines/{routine_id}/runs"),
        format!("/api/skills/{skill_id}"),
        format!("/api/mcp/servers/{mcp_id}"),
        format!("/api/threads/{thread_id}"),
        format!("/api/questions/{question_id}"),
    ];
    let writes = [
        ("PUT", format!("/api/kwants/{remy}"), bot("Remy")),
        ("PATCH", format!("/api/profiles/{profile_id}"), json!({"name": "Renamed"})),
        ("PATCH", format!("/api/skills/{skill_id}"), json!({"description": "Changed"})),
        ("POST", format!("/api/questions/{question_id}/answer"), json!({"choice": 0})),
    ];
    for path in &reads {
        let (s, v) = call(app, "GET", &format!("{path}?space={b}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "GET {path}?space={b}: {v}");
        ok(app, "GET", &format!("{path}?space=spc_default"), None).await;
        ok(app, "GET", path, None).await;
    }
    for (method, path, body) in &writes {
        let (s, v) = call(app, method, &format!("{path}?space={b}"), Some(body.clone())).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} {path}?space={b}: {v}");
    }
    // Nothing was changed by the refused writes
    let q = ok(app, "GET", &format!("/api/questions/{question_id}"), None).await;
    assert_eq!(q["status"].as_str().or(q["question"]["status"].as_str()), Some("open"), "{q}");
    let p = env.state.db.list_profiles(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert!(p.iter().all(|p| p.name != "Renamed"));
    // An unknown id under ?space= is still the handler's own 404
    assert_eq!(call(app, "GET", &format!("/api/teams/nope?space={b}"), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_skill_owner_must_be_in_the_requested_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let remy = env.bot("spc_default", "Remy").await;
    let bea = env.bot(&b, "Bea").await;
    let (s, v) = call(app, "POST", &format!("/api/skills?space={b}"), Some(skill("stray", Some(&remy)))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let made = ok(app, "POST", &format!("/api/skills?space={b}"), Some(skill("own", Some(&bea)))).await;
    assert_eq!(made["space_id"], b.as_str(), "{made}");
    // Without ?space= the owner still picks the space
    ok(app, "POST", "/api/skills", Some(skill("by-owner", Some(&bea)))).await;
}

#[tokio::test]
async fn cancel_without_ids_stops_only_the_requests_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let remy = env.bot("spc_default", "Remy").await;
    let bea = env.bot(&b, "Bea").await;
    let (a_cli, a_child) = sleepy_turn(&env, &remy, "cancelA").await;
    let (b_cli, b_child) = sleepy_turn(&env, &bea, "cancelB").await;

    ok(app, "POST", &format!("/api/chat/cancel?space={b}"), Some(json!({}))).await;
    wait_gone(&b_cli, "B's CLI").await;
    wait_gone(&b_child, "B's CLI child").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!gone(&a_cli), "Default's turn was stopped by B's cancel");
    assert!(env.state.turns.active_turns().iter().any(|t| t.kwant_id == remy));

    // No ?space= means Default
    ok(app, "POST", "/api/chat/cancel", Some(json!({}))).await;
    wait_gone(&a_cli, "Default's CLI").await;
    wait_gone(&a_child, "Default's CLI child").await;
}

#[tokio::test]
async fn answering_a_question_in_an_archived_space_is_refused_and_starts_no_turn() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let bea = env.bot(&b, "Bea").await;
    let q = env.question(&bea).await;
    ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": true}))).await;
    let (s, v) = call(app, "POST", &format!("/api/questions/{q}/answer"), Some(json!({"text": "ANSWER_IN_ARCHIVE"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(invocations_with("ANSWER_IN_ARCHIVE"), 0, "a turn was started");
    assert!(env.state.turns.active_turns().is_empty());
    let after = env.state.db.get_question(&q).await.unwrap().unwrap();
    assert_eq!(after.status, "open");
}

#[tokio::test]
async fn the_scheduler_skips_enabled_routines_of_an_archived_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let bea = env.bot(&b, "Bea").await;
    let r = ok(app, "POST", &format!("/api/routines?space={b}"), Some(routine(&bea, "digest", true))).await;
    let rid = r["id"].as_str().unwrap();
    ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": true}))).await;
    // Turned back on behind the archive's back, and due
    let now = chrono::Utc::now();
    env.state.db.set_routine_schedule_state(rid, Some(now - chrono::Duration::minutes(5)), true, None).await.unwrap();
    assert_eq!(kwnt::server::scheduler::tick(&env.state, now).await, 0);
    assert!(env.state.db.list_routine_runs(rid, 10).await.unwrap().is_empty());
    // Run now is refused as well
    assert_eq!(call(app, "POST", &format!("/api/routines/{rid}/run"), None).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn writes_on_an_archived_spaces_rows_are_refused() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let q = format!("?space={b}");
    let bea = env.bot(&b, "Bea").await;
    let team_id = ok(app, "POST", &format!("/api/teams{q}"), Some(team("beta", &bea, None))).await["id"].as_str().unwrap().to_string();
    let routine_id = ok(app, "POST", &format!("/api/routines{q}"), Some(routine(&bea, "digest", false))).await["id"].as_str().unwrap().to_string();
    let skill_id = ok(app, "POST", &format!("/api/skills{q}"), Some(skill("beta-notes", None))).await["id"].as_str().unwrap().to_string();
    let mcp_id = ok(app, "POST", &format!("/api/mcp/servers{q}"), Some(mcp("beta_mcp"))).await["id"].as_str().unwrap().to_string();
    let profile_id = ok(app, "POST", &format!("/api/profiles{q}"), Some(profile("Beta key"))).await["id"].as_str().unwrap().to_string();
    ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": true}))).await;

    let mut renamed_team = team("beta", &bea, None);
    renamed_team["id"] = json!(team_id);
    let writes = [
        ("PUT", format!("/api/kwants/{bea}"), Some(bot("Bea"))),
        ("POST", format!("/api/kwants/{bea}/memories"), Some(json!({"text": "remember"}))),
        ("PUT", format!("/api/teams/{team_id}"), Some(renamed_team)),
        ("DELETE", format!("/api/teams/{team_id}"), None),
        ("PATCH", format!("/api/routines/{routine_id}"), Some(json!({"name": "renamed"}))),
        ("PATCH", format!("/api/skills/{skill_id}"), Some(json!({"description": "Changed"}))),
        ("DELETE", format!("/api/skills/{skill_id}"), None),
        ("PUT", format!("/api/mcp/servers/{mcp_id}"), Some(mcp("beta_mcp"))),
        ("PATCH", format!("/api/profiles/{profile_id}"), Some(json!({"name": "Renamed"}))),
        ("POST", "/api/skills".to_string(), Some(skill("late", Some(&bea)))),
    ];
    for (method, path, body) in writes {
        // Deleting a team is person-only, so that row carries the API token
        let (s, v) = if method == "DELETE" && path.starts_with("/api/teams/") {
            call_as(app, method, &path, env.state.api_token.reveal(), body).await
        } else {
            call(app, method, &path, body).await
        };
        assert_eq!(s, StatusCode::CONFLICT, "{method} {path}: {v}");
    }
    // Reads still work, and nothing changed
    ok(app, "GET", &format!("/api/teams/{team_id}"), None).await;
    ok(app, "GET", &format!("/api/skills/{skill_id}"), None).await;
    assert_eq!(env.state.db.get_routine(&routine_id).await.unwrap().unwrap().name, "digest");
}

#[cfg(unix)]
#[tokio::test]
async fn archiving_kills_a_running_cli_and_its_children_and_posts_nothing() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let bea = env.bot(&b, "Bea").await;
    let (cli, child) = sleepy_turn(&env, &bea, "archive").await;
    assert!(!gone(&cli) && !gone(&child));

    ok(app, "PATCH", &format!("/api/spaces/{b}"), Some(json!({"archived": true}))).await;
    wait_gone(&cli, "the CLI").await;
    wait_gone(&child, "the CLI's sleeping child").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let posted: Vec<_> = env
        .state
        .db
        .list_messages(&bea, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.sender_role == "kwant")
        .map(|m| m.text)
        .collect();
    assert!(posted.iter().all(|t| !t.contains("answer ok")), "{posted:?}");
    assert!(env.state.turns.active_turns().is_empty());
}

#[tokio::test]
async fn chat_send_with_an_explicit_space_refuses_rows_of_another_space() {
    let env = setup().await;
    let app = &env.app;
    let b = env.new_space("Side").await;
    let bea = env.bot(&b, "Bea").await;
    env.bot(&b, "Bob").await;
    let remy = env.bot("spc_default", "Remy").await;
    let b_general = env.general(&b).await;
    let d_general = env.general("spc_default").await;
    let team_id = ok(app, "POST", "/api/teams", Some(team("alpha", &remy, None))).await["id"].as_str().unwrap().to_string();
    let thread_id = ok(app, "POST", &format!("/api/kwants/{remy}/threads"), Some(json!({"title": "t"}))).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let send = |target: Value, tag: &str| {
        let mut body = json!({"prompt": format!("hello STRAY_{tag}"), "sender_id": "user"});
        body.as_object_mut().unwrap().extend(target.as_object().unwrap().clone());
        body
    };

    // Both directions, every kind of target: 404, nothing saved, no CLI turn
    let cases = [
        (b.clone(), json!({"channel_id": d_general}), "dch"),
        (b.clone(), json!({"kwant_id": remy}), "dkw"),
        (b.clone(), json!({"team_id": team_id}), "dtm"),
        (b.clone(), json!({"thread_id": thread_id}), "dth"),
        ("spc_default".to_string(), json!({"channel_id": b_general}), "bch"),
        ("spc_default".to_string(), json!({"kwant_id": bea}), "bkw"),
    ];
    for (space, target, tag) in &cases {
        let (s, v) = call(app, "POST", &format!("/api/chat/send?space={space}"), Some(send(target.clone(), tag))).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "?space={space} {target}: {v}");
        assert_eq!(v["error"], "that isn't in this space", "{v}");
    }
    // A channel of this space with a kwant of the other is refused too
    let (s, _) = call(app, "POST", &format!("/api/chat/send?space={b}"), Some(send(json!({"channel_id": b_general, "kwant_id": remy}), "mix"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // An unknown space is the usual 404
    let (s, _) = call(app, "POST", "/api/chat/send?space=nope", Some(send(json!({"kwant_id": remy}), "unk"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    tokio::time::sleep(Duration::from_millis(300)).await;
    for ctx in [&b_general, &d_general, &remy, &bea, &thread_id] {
        let msgs = ok(app, "GET", &format!("/api/channels/{ctx}/messages"), None).await;
        let text = msgs.to_string();
        assert!(!text.contains("STRAY_"), "a refused send was saved in {ctx}: {text}");
    }
    assert_eq!(invocations_with("STRAY_"), 0, "a refused send ran a CLI turn");

    // Matching or missing ?space= still sends
    ok(app, "POST", &format!("/api/chat/send?space={b}"), Some(send(json!({"kwant_id": bea}), "okb"))).await;
    ok(app, "POST", "/api/chat/send?space=spc_default", Some(send(json!({"kwant_id": remy}), "okd"))).await;
    ok(app, "POST", "/api/chat/send", Some(send(json!({"kwant_id": bea}), "okn"))).await;
    env.idle().await;
}

#[tokio::test]
async fn server_messages_say_bot_and_general_describes_the_space() {
    let env = setup().await;
    let app = &env.app;
    let general = env.general("spc_default").await;
    let c = ok(app, "GET", &format!("/api/channels/{general}"), None).await;
    let desc = c["description"].as_str().or(c["channel"]["description"].as_str()).unwrap_or_default().to_string();
    assert_eq!(desc, kwnt::db::GENERAL_DESCRIPTION, "{c}");

    // An unedited old seed text is replaced on the next start; an edited one is kept
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", env.data.join("kwnt.db").display())).await.unwrap();
    sqlx::query("UPDATE channels SET description = 'Main collaboration hub for all agents' WHERE id = ?")
        .bind(&general)
        .execute(&pool)
        .await
        .unwrap();
    Database::init(&env.data).await.unwrap();
    let (d,): (String,) = sqlx::query_as("SELECT description FROM channels WHERE id = ?").bind(&general).fetch_one(&pool).await.unwrap();
    assert_eq!(d, kwnt::db::GENERAL_DESCRIPTION);
    sqlx::query("UPDATE channels SET description = 'Ours' WHERE id = ?").bind(&general).execute(&pool).await.unwrap();
    Database::init(&env.data).await.unwrap();
    let (d,): (String,) = sqlx::query_as("SELECT description FROM channels WHERE id = ?").bind(&general).fetch_one(&pool).await.unwrap();
    assert_eq!(d, "Ours");

    let (s, v) = call(app, "GET", "/api/kwants/nope/mcp-servers", None).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("bot not found")), "{v}");
}
