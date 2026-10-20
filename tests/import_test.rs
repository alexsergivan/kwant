//! Bringing bots in from other apps (`/api/import/*`), undoing an import, and exporting bots as
//! `kwant.bundle/1`. Preview writes nothing; apply creates bots with no tools, pending skills and
//! paused routines; undo removes exactly what an import made unless it has been used; hostile
//! inputs (zip bombs, paths out of the archive, keys) are refused or cleaned.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{McpServer, NewMessage},
        spaces::{NewSpace, SpaceId},
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    tmp: TempDir,
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
    Env { app: create_router(state.clone()), state, tmp }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

async fn send(app: &Router, method: &str, uri: &str, content_type: &str, body: Vec<u8>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", content_type).body(Body::from(body)).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Undoing an import is person-only: it carries the API token.
async fn undo_call(e: &Env, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", e.state.api_token.reveal()))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn json_call(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    send(app, method, uri, "application/json", body.to_string().into_bytes()).await
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    send(app, "GET", uri, "application/json", vec![]).await
}

/// Preview a fixture file as an upload.
async fn preview_file(app: &Router, name: &str, space: &str) -> (StatusCode, Value) {
    let file = name.rsplit('/').next().unwrap();
    send(app, "POST", &format!("/api/import/preview?space={space}&filename={file}"), "application/octet-stream", fixture(name)).await
}

async fn apply(app: &Router, space: &str, bundle: &Value, skip: &[&str]) -> (StatusCode, Value) {
    json_call(app, "POST", &format!("/api/import/apply?space={space}"), json!({ "bundle": bundle, "skip": skip })).await
}

/// Rows in every table an import could touch, straight from the database file.
async fn row_counts(tmp: &TempDir) -> Vec<(String, i64)> {
    let url = format!("sqlite://{}?mode=ro", tmp.path().join("kwnt.db").display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let mut out = Vec::new();
    for t in ["kwants", "skills", "routines", "kwant_memories", "teams", "channels", "kwant_mcp_disabled", "imports", "attachments"] {
        let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {t}")).fetch_one(&pool).await.unwrap();
        out.push((t.to_string(), n));
    }
    pool.close().await;
    out
}

fn bot_names(preview: &Value) -> Vec<String> {
    preview["bundle"]["bots"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap().to_string()).collect()
}

fn dflt() -> String {
    SpaceId::default_space().as_str().to_string()
}

async fn new_space(e: &Env, name: &str) -> String {
    let s = e
        .state
        .db
        .create_space(&NewSpace { name: name.into(), icon: "box".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    s.id.as_str().to_string()
}

#[tokio::test]
async fn every_parser_previews_its_fixture_without_writing() {
    let e = setup().await;
    let before = row_counts(&e.tmp).await;
    let cases: [(&str, &str, &[&str]); 8] = [
        ("claude-code-agent.md", "claude-code", &["code-reviewer"]),
        ("SKILL.md", "skill", &[]),
        ("card-v2.json", "character-card", &["Captain-Quill"]),
        ("card-v3.png", "character-card", &["Lumen"]),
        ("grokbot-bundle.json", "grokbot", &["Market-Scout"]),
        ("openmaus-package.json", "openmaus", &["Ada-Fixture", "Rigel-Fixture"]),
        ("kwant-bundle.json", "kwant", &["Helper"]),
        ("prompt-reply.txt", "chatgpt", &["Recipe-Pal"]),
    ];
    for (file, kind, names) in cases {
        let (status, v) = preview_file(&e.app, file, &dflt()).await;
        assert_eq!(status, StatusCode::OK, "{file}: {v}");
        assert_eq!(v["bundle"]["source"]["kind"], kind, "{file}");
        assert_eq!(bot_names(&v), names, "{file}");
        assert_eq!(v["can_apply"], true, "{file}: {v}");
    }
    // The pasted form of the export-prompt reply.
    let text = String::from_utf8(fixture("prompt-reply.txt")).unwrap();
    let (status, v) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": text })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(bot_names(&v), ["Recipe-Pal"]);
    assert_eq!(v["bundle"]["bots"][0]["routines"][0]["schedule"]["type"], "cron");

    let (_, card) = preview_file(&e.app, "card-v2.json", &dflt()).await;
    let persona = card["bundle"]["bots"][0]["persona_prompt"].as_str().unwrap();
    assert!(persona.contains("Captain Quill is a retired ship captain") && !persona.contains("{{"), "{persona}");
    let (_, skill) = preview_file(&e.app, "SKILL.md", &dflt()).await;
    assert_eq!(skill["bundle"]["skills"][0]["name"], "release-notes");
    let (_, pkg) = preview_file(&e.app, "openmaus-package.json", &dflt()).await;
    assert_eq!(pkg["bundle"]["teams"][0]["name"], "Fixture Engineering");
    assert_eq!(pkg["bundle"]["bots"][0]["memories"].as_array().unwrap().len(), 2);

    assert_eq!(row_counts(&e.tmp).await, before, "preview must not write anything");
}

#[tokio::test]
async fn apply_creates_bots_without_tools_pending_skills_and_paused_routines_then_undo_removes_them() {
    let e = setup().await;
    let space = SpaceId::default_space();
    let mut srv = McpServer {
        id: String::new(),
        name: "files".into(),
        transport: "stdio".into(),
        command: Some("/usr/bin/true".into()),
        args: vec![],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 30,
        created_at: None,
    };
    srv = e.state.db.create_mcp_server(&space, &srv).await.unwrap();
    let before = row_counts(&e.tmp).await;

    let (_, preview) = preview_file(&e.app, "kwant-bundle.json", &dflt()).await;
    let (status, applied) = apply(&e.app, &dflt(), &preview["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let import_id = applied["import"]["id"].as_str().unwrap().to_string();
    let bot_id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap().to_string();

    let bot = e.state.db.get_kwant(&bot_id).await.unwrap().unwrap();
    assert_eq!((bot.name.as_str(), bot.preferred_model.as_str()), ("Helper", "auto"));
    assert!(e.state.db.list_active_mcp_servers_for_kwant(&bot_id).await.unwrap().is_empty(), "imported bots get no tools");
    let skills = e.state.db.list_skills(&space, Some(&bot_id), None).await.unwrap();
    let mine: Vec<_> = skills.iter().filter(|s| s.kwant_id.as_deref() == Some(bot_id.as_str())).collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].status, "pending");
    let routines: Vec<_> = e.state.db.list_routines(&space).await.unwrap().into_iter().filter(|r| r.kwant_id == bot_id).collect();
    assert_eq!(routines.len(), 1);
    assert!(!routines[0].enabled && routines[0].next_run_at.is_none(), "routines arrive paused");
    assert_eq!(e.state.db.list_memories(&bot_id).await.unwrap().len(), 1);
    assert!(!srv.id.is_empty(), "the space had a tool the import had to opt the bot out of");

    let (status, list) = get(&e.app, "/api/imports").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["id"], import_id.as_str());

    let (status, undone) = undo_call(&e, &format!("/api/imports/{import_id}/undo"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{undone}");
    assert_eq!(undone["status"], "undone");
    let mut after = row_counts(&e.tmp).await;
    let mut expected = before.clone();
    after.retain(|(t, _)| t != "imports");
    expected.retain(|(t, _)| t != "imports");
    assert_eq!(after, expected, "undo removes everything the import created");

    let (status, again) = undo_call(&e, &format!("/api/imports/{import_id}/undo"), json!({})).await;
    assert_eq!((status, again["code"].as_str()), (StatusCode::CONFLICT, Some("already_undone")));
}

#[tokio::test]
async fn name_clash_renames_and_never_overwrites() {
    let e = setup().await;
    let existing = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let bundle = json!({
        "format": "kwant.bundle/1",
        "source": { "kind": "other" },
        "bots": [{ "ref": "a", "name": existing.name.to_uppercase(), "title": "Impostor", "persona_prompt": "New text." }]
    });
    let (status, preview) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": bundle.to_string() })).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let renamed = format!("{}-2", existing.name.to_uppercase());
    assert_eq!(bot_names(&preview), std::slice::from_ref(&renamed));
    assert_eq!(preview["renames"][0]["to"], renamed.as_str());

    // Apply re-checks names itself, so even the raw bundle can't overwrite.
    let (status, applied) = apply(&e.app, &dflt(), &bundle, &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    assert_eq!(applied["import"]["created"]["kwants"][0]["name"], renamed.as_str());
    let still = e.state.db.get_kwant(&existing.id).await.unwrap().unwrap();
    assert_eq!((still.name, still.persona_prompt), (existing.name, existing.persona_prompt));
}

#[tokio::test]
async fn package_with_team_lands_in_a_new_space_and_undo_removes_team_channels_and_general() {
    let e = setup().await;
    let space = new_space(&e, "Imported").await;
    let (_, preview) = preview_file(&e.app, "openmaus-package.json", &space).await;
    let (status, applied) = apply(&e.app, &space, &preview["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let created = &applied["import"]["created"];
    assert_eq!(created["kwants"].as_array().unwrap().len(), 2);
    assert_eq!(created["teams"].as_array().unwrap().len(), 1);
    let sid = SpaceId::new(space.clone());
    let channels: Vec<String> = e.state.db.list_channels(&sid).await.unwrap().into_iter().map(|c| c.name).collect();
    assert!(channels.contains(&"general".to_string()) && channels.contains(&"team-Fixture Engineering".to_string()), "{channels:?}");
    let teams = e.state.db.list_teams(&sid).await.unwrap();
    assert_eq!(teams[0].instructions.as_deref(), Some("Ship safely."));
    // Shared skill + 2 for the lead (skill and playbook), all pending.
    let skills = e.state.db.list_skills(&sid, None, None).await.unwrap();
    assert_eq!(skills.len(), 3);
    assert!(skills.iter().all(|s| s.status == "pending"));

    // Undo needs the right space.
    let id = applied["import"]["id"].as_str().unwrap();
    let (status, _) = undo_call(&e, &format!("/api/imports/{id}/undo"), Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an import in another space isn't visible from Default");
    let (status, v) = undo_call(&e, &format!("/api/imports/{id}/undo?space={space}"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(e.state.db.list_kwants(&sid).await.unwrap().is_empty());
    assert!(e.state.db.list_teams(&sid).await.unwrap().is_empty());
    assert!(e.state.db.list_channels(&sid).await.unwrap().is_empty());
    assert!(e.state.db.list_skills(&sid, None, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn skip_leaves_bots_out() {
    let e = setup().await;
    let space = new_space(&e, "Skip").await;
    let (_, preview) = preview_file(&e.app, "openmaus-package.json", &space).await;
    let (status, applied) = apply(&e.app, &space, &preview["bundle"], &["qa"]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let names: Vec<&str> = applied["import"]["created"]["kwants"].as_array().unwrap().iter().map(|k| k["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Ada-Fixture"]);
}

#[tokio::test]
async fn undo_is_refused_once_a_bot_is_used_and_needs_force_once_edited() {
    let e = setup().await;
    let (_, preview) = preview_file(&e.app, "kwant-bundle.json", &dflt()).await;
    let (_, applied) = apply(&e.app, &dflt(), &preview["bundle"], &[]).await;
    let id = applied["import"]["id"].as_str().unwrap().to_string();
    let bot_id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap().to_string();

    // Edited: refused, then removed with force.
    let (status, v) = json_call(
        &e.app,
        "PUT",
        &format!("/api/kwants/{bot_id}"),
        json!({ "name": "Helper", "title": "Edited", "persona_prompt": "Changed.", "soul": "", "avatar_sprite": "default", "preferred_model": "auto" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (status, v) = undo_call(&e, &format!("/api/imports/{id}/undo"), Value::Null).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("edited")), "{v}");
    assert_eq!(v["edited"][0], "Helper");

    // Used: a direct message. Refused even with force; nothing removed.
    e.state
        .db
        .insert_message(&NewMessage {
            channel_id: None,
            direct_kwant_id: Some(bot_id.clone()),
            sender_role: "user".into(),
            sender_id: "user".into(),
            text: "hi".into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        })
        .await
        .unwrap();
    let (status, v) = undo_call(&e, &format!("/api/imports/{id}/undo"), json!({ "force": true })).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("in_use")), "{v}");
    assert!(e.state.db.get_kwant(&bot_id).await.unwrap().is_some());
}

#[tokio::test]
async fn forced_undo_removes_an_edited_bot() {
    let e = setup().await;
    let (_, preview) = preview_file(&e.app, "grokbot-bundle.json", &dflt()).await;
    let (_, applied) = apply(&e.app, &dflt(), &preview["bundle"], &[]).await;
    let id = applied["import"]["id"].as_str().unwrap().to_string();
    let bot_id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap().to_string();
    let (status, _) = json_call(
        &e.app,
        "PUT",
        &format!("/api/kwants/{bot_id}"),
        json!({ "name": "Market-Scout", "title": "Edited", "persona_prompt": "x", "soul": "", "avatar_sprite": "data", "preferred_model": "auto" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, v) = undo_call(&e, &format!("/api/imports/{id}/undo"), json!({ "force": true })).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(e.state.db.get_kwant(&bot_id).await.unwrap().is_none());
}

/// Export → preview → apply into another space → export: the bots come back the same.
#[tokio::test]
async fn export_import_round_trip() {
    let e = setup().await;
    let a = new_space(&e, "Source").await;
    let b = new_space(&e, "Copy").await;
    for f in ["openmaus-package.json", "kwant-bundle.json", "prompt-reply.txt"] {
        let (_, p) = preview_file(&e.app, f, &a).await;
        let (status, v) = apply(&e.app, &a, &p["bundle"], &[]).await;
        assert_eq!(status, StatusCode::CREATED, "{f}: {v}");
    }
    let (status, first) = get(&e.app, &format!("/api/export?space={a}")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["format"], "kwant.bundle/1");
    assert_eq!(first["bots"].as_array().unwrap().len(), 4);

    let (status, p) = send(&e.app, "POST", &format!("/api/import/preview?space={b}&filename=space.kwant.json"), "application/octet-stream", first.to_string().into_bytes()).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert!(p["renames"].as_array().unwrap().is_empty(), "{p}");
    let (status, v) = apply(&e.app, &b, &p["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let (_, second) = get(&e.app, &format!("/api/export?space={b}")).await;

    // Everything but where and when it came from is the same.
    let strip = |mut v: Value| {
        v["source"] = Value::Null;
        v
    };
    assert_eq!(strip(first.clone()), strip(second.clone()));

    // A single bot exports the same way, and the row guard keeps it to its space.
    let kid = e.state.db.list_kwants(&SpaceId::new(a.clone())).await.unwrap()[0].id.clone();
    let (status, one) = get(&e.app, &format!("/api/kwants/{kid}/export?space={a}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["bots"].as_array().unwrap().len(), 1);
    let (status, _) = get(&e.app, &format!("/api/kwants/{kid}/export?space={b}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hostile_archives_are_refused() {
    let e = setup().await;
    let before = row_counts(&e.tmp).await;
    let cases = [
        ("hostile/zip-bomb-one-file.zip", StatusCode::PAYLOAD_TOO_LARGE),
        ("hostile/zip-bomb-total.zip", StatusCode::PAYLOAD_TOO_LARGE),
        ("hostile/zip-slip.zip", StatusCode::UNPROCESSABLE_ENTITY),
        ("hostile/zip-absolute.zip", StatusCode::UNPROCESSABLE_ENTITY),
        ("hostile/zip-backslash.zip", StatusCode::UNPROCESSABLE_ENTITY),
        ("hostile/zip-nested.zip", StatusCode::UNPROCESSABLE_ENTITY),
        ("hostile/zip-too-many-files.zip", StatusCode::UNPROCESSABLE_ENTITY),
    ];
    for (file, want) in cases {
        let (status, v) = preview_file(&e.app, file, &dflt()).await;
        assert_eq!(status, want, "{file}: {v}");
        assert!(v["error"].as_str().is_some_and(|m| !m.is_empty()), "{file}: {v}");
    }
    assert!(!e.tmp.path().join("evil.md").exists());
    assert_eq!(row_counts(&e.tmp).await, before, "nothing written, nothing stored");
}

#[tokio::test]
async fn oversize_and_unknown_inputs_are_refused() {
    let e = setup().await;
    let big = vec![b'a'; 21 * 1024 * 1024];
    let (status, _) = send(&e.app, "POST", "/api/import/preview?filename=x.json", "application/octet-stream", big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let mid = vec![b'a'; 5 * 1024 * 1024];
    let (status, _) = send(&e.app, "POST", "/api/import/preview?filename=x.md", "application/octet-stream", mid).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, _) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": "x".repeat(300 * 1024) })).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, v) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": " \n\t " })).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid")), "{v}");
    // A reply that means to be JSON but is broken keeps its error rather than becoming instructions.
    let (status, v) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": "Here you go:\n```json\n{ \"format\": \"kwant.bundle/1\", \"bots\": [ }\n```" })).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid")), "{v}");
    let (status, _) = send(&e.app, "POST", "/api/import/preview", "application/octet-stream", vec![0xff, 0xfe, 0x00]).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (status, _) = json_call(&e.app, "POST", "/api/import/apply", json!({ "bundle": { "format": "something/else" } })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn yaml_alias_bomb_does_not_hang_or_blow_up() {
    let e = setup().await;
    let started = std::time::Instant::now();
    let (status, v) = preview_file(&e.app, "hostile/yaml-alias-bomb.md", &dflt()).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    assert!(status.is_client_error(), "{status}: {v}");
}

#[tokio::test]
async fn keys_are_scrubbed_before_preview_and_never_stored() {
    let e = setup().await;
    let (status, preview) = preview_file(&e.app, "hostile/keys.json", &dflt()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let text = preview["bundle"].to_string();
    assert!(!text.contains("FAKEFAKE"), "a key survived: {text}");
    assert!(!text.contains("Hunter2Hunter2x9"), "{text}");
    assert!(text.contains("[removed key]"));
    assert!(preview["warnings"].to_string().contains("key"), "{}", preview["warnings"]);

    // Even a bundle sent straight to apply is scrubbed.
    let raw: Value = serde_json::from_slice(&fixture("hostile/keys.json")).unwrap();
    let (status, applied) = apply(&e.app, &dflt(), &raw, &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap();
    let (_, exported) = get(&e.app, &format!("/api/kwants/{id}/export")).await;
    assert!(!exported.to_string().contains("FAKEFAKE"), "{exported}");
}

#[tokio::test]
async fn prompt_injection_is_flagged_not_executed() {
    let e = setup().await;
    let (status, v) = preview_file(&e.app, "hostile/prompt-injection-reply.txt", &dflt()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    // `bot` is the bot's ref in the bundle.
    let warnings: Vec<&str> = v["warnings"].as_array().unwrap().iter().filter(|w| w["bot"] == "x").map(|w| w["message"].as_str().unwrap()).collect();
    assert!(warnings.iter().any(|w| w.contains("ignore earlier instructions")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("send data somewhere")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("invisible whitespace")), "{warnings:?}");
    // Flagged, not changed: the person decides.
    assert!(v["bundle"]["bots"][0]["persona_prompt"].as_str().unwrap().starts_with("Ignore previous"));
}

#[tokio::test]
async fn persona_and_soul_are_capped_at_16k_everywhere() {
    let e = setup().await;
    let long = "x".repeat(16_001);
    let ok = "x".repeat(16_000);
    let body = |persona: &str, soul: &str| {
        json!({ "name": "Capped", "title": "t", "persona_prompt": persona, "soul": soul, "avatar_sprite": "default", "driver_type": "api", "preferred_model": "auto" })
    };
    let (status, v) = json_call(&e.app, "POST", "/api/kwants", body(&long, "")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], "instructions can be at most 16,000 characters");
    let (status, v) = json_call(&e.app, "POST", "/api/kwants", body("", &long)).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("soul can be at most 16,000 characters")));
    let (status, created) = json_call(&e.app, "POST", "/api/kwants", body(&ok, &ok)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().or(created["kwant"]["id"].as_str()).unwrap().to_string();
    let (status, v) = json_call(&e.app, "PUT", &format!("/api/kwants/{id}"), body(&long, "")).await;
    assert_eq!((status, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("instructions can be at most 16,000 characters")));

    // Import: a problem in preview, and apply refuses it.
    let bundle = json!({ "format": "kwant.bundle/1", "source": { "kind": "other" }, "bots": [{ "ref": "a", "name": "Long", "persona_prompt": long }] });
    let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": bundle.to_string() })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(p["can_apply"], false);
    assert!(p["problems"][0]["message"].as_str().unwrap().contains("16,000"));
    let (status, v) = apply(&e.app, &dflt(), &bundle, &[]).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("problems")), "{v}");
}

#[tokio::test]
async fn export_prompts_start_with_grok_chatgpt_claude() {
    let e = setup().await;
    let (status, v) = get(&e.app, "/api/import/prompts").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = v.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["grok", "chatgpt", "claude", "other"]);
    assert!(v[0]["prompt"].as_str().unwrap().contains("kwant.bundle/1"));
}

#[tokio::test]
async fn plain_text_becomes_one_bots_instructions_scrubbed_and_capped() {
    let e = setup().await;
    let before = row_counts(&e.tmp).await;
    let text = "You are a friendly travel planner. Ask where {city} is before suggesting anything.\n\nKeep answers short.\nOur key is sk-ant-api03-FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE.";
    let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": text })).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(p["can_apply"], true, "{p}");
    assert_eq!(p["bundle"]["source"]["kind"], "plain-text");
    assert_eq!(bot_names(&p), ["Imported-bot"]);
    let persona = p["bundle"]["bots"][0]["persona_prompt"].as_str().unwrap();
    assert!(persona.starts_with("You are a friendly travel planner. Ask where {city} is"), "{persona}");
    assert!(!persona.contains("FAKEFAKE") && persona.contains("[removed key]"), "{persona}");
    let warnings = p["warnings"].to_string();
    assert!(warnings.contains("No name or memory found"), "{warnings}");
    assert!(warnings.contains("key"), "{warnings}");
    assert!(!warnings.contains("can't have spaces"), "the default name needs no cleaning: {warnings}");
    assert_eq!(row_counts(&e.tmp).await, before, "preview writes nothing");

    let (status, v) = apply(&e.app, &dflt(), &p["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // A second paste gets a new name instead of overwriting the first.
    let (_, again) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": "Be brief." })).await;
    assert_eq!(bot_names(&again), ["Imported-bot-2"]);

    // Over 16,000 characters: a problem in preview, nothing to apply.
    let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": "word ".repeat(4_000) })).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(p["can_apply"], false);
    assert!(p["problems"][0]["message"].as_str().unwrap().contains("16,000"), "{p}");
}

#[tokio::test]
async fn space_export_is_named_after_the_space() {
    let e = setup().await;
    let named = new_space(&e, "Travel Team!").await;
    let symbols = new_space(&e, "★★★").await;
    for (space, want) in [(dflt(), "Default.kwant.json".to_string()), (named, "Travel-Team.kwant.json".into()), (symbols.clone(), format!("{symbols}.kwant.json"))] {
        let req = Request::builder().uri(format!("/api/export?space={space}")).body(Body::empty()).unwrap();
        let resp = e.app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cd = resp.headers()["content-disposition"].to_str().unwrap().to_string();
        assert_eq!(cd, format!("attachment; filename=\"{want}\""));
    }
}

/// Every byte under the data dir, as lossy text, for "is this anywhere on disk" checks.
fn everything_on_disk(tmp: &TempDir) -> String {
    let mut out = String::new();
    let mut dirs = vec![tmp.path().to_path_buf()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(d).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                dirs.push(p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    out
}

/// IMPORT-QA.md F1: secret shapes the first scrubber missed, in every field, never reach the
/// preview, the database or an export.
#[tokio::test]
async fn qa_f1_pem_passwords_url_credentials_aws_npm_and_split_keys_are_removed() {
    let e = setup().await;
    let secrets = [
        "FAKEPEMBODY",
        "p@ssFAKE!Secret#99",
        "SuperFAKESecret123",
        "Tr0ubFAKE&3xyz",
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "npm_FAKE",
        "ya29.FAKE",
        "FAKEtailFAKE",
        "FAKEs3cret!value",
        "FAKEdbPass99",
    ];
    let (status, preview) = preview_file(&e.app, "hostile/keys-qa.json", &dflt()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let text = preview["bundle"].to_string();
    for s in secrets {
        assert!(!text.contains(s), "{s} survived the preview: {text}");
    }
    // Hosts stay readable; only the credentials go.
    assert!(text.contains("https://[removed key]@git.example.com/repo.git"), "{text}");
    assert!(text.contains("postgres://[removed key]@db.example.com:5432/app"), "{text}");

    let (status, applied) = apply(&e.app, &dflt(), &preview["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap();
    let (_, exported) = get(&e.app, &format!("/api/kwants/{id}/export")).await;
    let on_disk = everything_on_disk(&e.tmp);
    for s in secrets {
        assert!(!exported.to_string().contains(s), "{s} in the export");
        assert!(!on_disk.contains(s), "{s} stored on disk");
    }
}

/// IMPORT-QA.md F2: instructions that mention "format", start with `{` or show a JSON example
/// import as plain text; pasted text never gets a 415; a broken export reply keeps its 422.
#[tokio::test]
async fn qa_f2_instruction_like_text_imports_as_plain_text() {
    let e = setup().await;
    for text in [
        "Always answer in the \"format\" the user asks for.",
        "I like the kwant.bundle/1 format.",
        "{Be nice} to everyone.",
        "When asked for data, reply like this:\n```json\n{\"a\": 1}\n```\nand nothing else.",
    ] {
        let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": text })).await;
        assert_eq!(status, StatusCode::OK, "{text:?}: {p}");
        assert_eq!(p["bundle"]["source"]["kind"], "plain-text", "{text:?}");
        assert_eq!(p["bundle"]["bots"][0]["persona_prompt"], text);
        assert_eq!(p["can_apply"], true);
    }
    for broken in [
        "Here you go:\n```json\n{\"format\": \"kwant.bundle/1\", \"bots\": [{\"name\": \"A\",}\n```",
        "```json\n{\"bots\": [{\"name\": \"A\"}]}\n```",
    ] {
        let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": broken })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{broken:?}: {p}");
    }
}

/// IMPORT-QA.md F4 and F5: the over-16k problem says where to shorten it, and hidden or
/// direction characters never reach a bot, team or routine name.
#[tokio::test]
async fn qa_f4_f5_cap_wording_and_hidden_characters_in_names() {
    let e = setup().await;
    let (_, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": "x".repeat(16_001) })).await;
    let problem = p["problems"][0]["message"].as_str().unwrap();
    assert!(problem.contains("Shorten it where it came from") && !problem.contains("here before"), "{problem}");

    let (status, preview) = preview_file(&e.app, "hostile/keys-qa.json", &dflt()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let (status, applied) = apply(&e.app, &dflt(), &preview["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let sid = SpaceId::default_space();
    let bot = e.state.db.list_kwants(&sid).await.unwrap().into_iter().find(|k| k.name.starts_with("Evil")).unwrap();
    assert_eq!(bot.name, "EvilName");
    assert!(preview["warnings"].to_string().contains("We removed hidden characters from the name EvilName"), "{}", preview["warnings"]);
    assert_eq!(bot.title, "Has fake secrets everywhere");
    let teams = e.state.db.list_teams(&sid).await.unwrap();
    assert!(teams.iter().any(|t| t.name == "Ops Team"), "{:?}", teams.iter().map(|t| &t.name).collect::<Vec<_>>());
    let routines = e.state.db.list_routines(&sid).await.unwrap();
    assert!(routines.iter().any(|r| r.name == "Backup"), "{:?}", routines.iter().map(|r| &r.name).collect::<Vec<_>>());
}

/// Oleks's first real import (2026-10-19): "0 7 * * * (Europe/Berlin)" is a valid cron line with
/// a zone, and used to arrive as daily 09:00 UTC with a "couldn't read" warning.
#[tokio::test]
async fn pasted_schedule_with_a_time_zone_keeps_it_and_stays_paused() {
    let e = setup().await;
    let reply = json!({
        "format": "kwant.bundle/1",
        "source": { "kind": "other", "label": "Personal" },
        "bots": [{
            "ref": "main",
            "name": "Personal",
            "persona_prompt": "Give me an overview every morning.",
            "routines": [
                { "name": "Morning overview", "instructions": "Check mail, calendar and weather.", "schedule_text": "0 7 * * * (Europe/Berlin)" },
                { "name": "Evening", "instructions": "Sum up the day.", "schedule": { "type": "cron", "expression": "0 19 * * *", "timeZone": "Europe/Berlin" } },
                { "name": "Lunch", "instructions": "Remind me to eat.", "schedule_text": "0 12 * * * (Mars/Olympus)" }
            ]
        }]
    });
    let text = format!("Here you go:\n```json\n{reply}\n```");
    let (status, p) = json_call(&e.app, "POST", "/api/import/preview", json!({ "text": text })).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    let routines = &p["bundle"]["bots"][0]["routines"];
    assert_eq!(routines[0]["schedule"], json!({ "type": "cron", "expression": "0 7 * * *", "tz": "Europe/Berlin" }), "{p}");
    assert_eq!(routines[1]["schedule"], json!({ "type": "cron", "expression": "0 19 * * *", "tz": "Europe/Berlin" }), "{p}");
    assert_eq!(routines[2]["schedule"], json!({ "type": "cron", "expression": "0 12 * * *", "tz": "UTC" }), "{p}");
    let warnings = p["warnings"].to_string();
    assert!(!warnings.contains("couldn't read"), "{warnings}");
    assert!(warnings.contains("Routine Lunch: we don't know the time zone") && warnings.contains("Mars/Olympus"), "{warnings}");
    assert!(!warnings.contains("Routine Morning overview") && !warnings.contains("Routine Evening"), "{warnings}");

    let (status, applied) = apply(&e.app, &dflt(), &p["bundle"], &[]).await;
    assert_eq!(status, StatusCode::CREATED, "{applied}");
    let bot_id = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap().to_string();
    let rows: Vec<_> = e.state.db.list_routines(&SpaceId::default_space()).await.unwrap().into_iter().filter(|r| r.kwant_id == bot_id).collect();
    let morning = rows.iter().find(|r| r.name == "Morning overview").unwrap();
    let stored: Value = serde_json::from_str(&morning.schedule).unwrap();
    assert_eq!(stored, json!({ "type": "cron", "expression": "0 7 * * *", "tz": "Europe/Berlin" }));
    assert!(rows.iter().all(|r| !r.enabled && r.next_run_at.is_none()), "routines still arrive paused");
}
