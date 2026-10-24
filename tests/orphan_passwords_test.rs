//! Saved passwords don't outlive their owner unseen (audit 2026-10-20, F-11). Deleting a bot left
//! its `kwnt-vault/<id>.bin` behind, and `local_count` only counted live owners, so approvals
//! could go off while a bot could still decrypt that file with the built-in key.
//!
//! - Deleting a bot, team or channel deletes its file in the built-in store.
//! - `local_count` (and with it `exposed` and every approvals-off check) counts every file in the
//!   built-in store, so a leftover from an older install blocks approvals off.
//! - An archived space keeps its bots, and their passwords keep counting.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{NewChannel, NewKwant, NewTeam},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    sandbox::credentials::VAULT_DIR,
    server::{create_router, passwords::forget_deleted_owners, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// An owner id that is not a team, channel or bot: a leftover from before deletes removed files.
const GONE: &str = "0b5e1e7e-dead-4bad-8bad-000000000011";

struct Env {
    app: Router,
    state: AppState,
    vault: PathBuf,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    assert_eq!(state.credentials.store_name(), "local");
    Env { app: create_router(state.clone()), state, vault: data.join(VAULT_DIR), _tmp: tmp }
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
    async fn bots(&self) -> Vec<String> {
        self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().into_iter().map(|k| k.id).collect()
    }
    async fn save_for(&self, owner: &str) {
        let body = json!({"origin": "https://orphan.test", "username": "ou", "password": "DUMMY-PW-ORPHAN"});
        let (s, v) = self.person("POST", &format!("/api/sandbox/{owner}/passwords"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    fn file(&self, owner: &str) -> PathBuf {
        self.vault.join(format!("{owner}.bin"))
    }
    async fn local_count(&self) -> Value {
        self.person("GET", "/api/passwords/store", None).await.1["local_count"].clone()
    }
    async fn bot_off(&self, bot: &str) -> (StatusCode, Value) {
        self.person("PUT", &format!("/api/kwants/{bot}/approvals"), Some(json!({"approvals": "off"}))).await
    }
    async fn space_off(&self) -> (StatusCode, Value) {
        let uri = format!("/api/spaces/{}/approvals", SpaceId::default_space().as_str());
        self.person("PUT", &uri, Some(json!({"approvals": "off"}))).await
    }
}

fn assert_refused(s: StatusCode, v: &Value, count: u64) {
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_bitwarden"], true, "{v}");
    assert_eq!(v["local_count"], count, "{v}");
}

/// Rigel's repro: save for a bot, delete the bot. Its file goes with it, so nothing is left for a
/// bot to decrypt, and approvals may go off.
#[tokio::test]
async fn deleting_a_bot_deletes_its_saved_passwords() {
    let e = setup().await;
    let bots = e.bots().await;
    let (doomed, other) = (&bots[0], &bots[1]);
    e.save_for(doomed).await;
    assert!(e.file(doomed).exists());
    assert_eq!(e.local_count().await, 1);

    let (s, v) = e.person("DELETE", &format!("/api/kwants/{doomed}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.file(doomed).exists(), "the deleted bot's file is removed");
    assert_eq!(e.local_count().await, 0);
    let (s, v) = e.bot_off(other).await;
    assert_eq!(s, StatusCode::OK, "nothing is left to read: {v}");
}

/// A leftover file whose owner no longer exists (an older install) counts: approvals off is
/// refused for a bot and for the space, `exposed` shows, and saving is refused once off.
#[tokio::test]
async fn a_leftover_file_of_a_deleted_owner_counts() {
    let e = setup().await;
    e.state.credentials.local().save(GONE, "https://orphan.test", "ou", "DUMMY-PW-ORPHAN").await.unwrap();
    assert!(e.file(GONE).exists());
    assert_eq!(e.local_count().await, 1, "a deleted owner's file counts");

    let bot = e.bots().await.remove(0);
    let (s, v) = e.bot_off(&bot).await;
    assert_refused(s, &v, 1);
    let (s, v) = e.space_off().await;
    assert_refused(s, &v, 1);

    // An install already in that state is reported, not changed
    e.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();
    let (_, store) = e.person("GET", "/api/passwords/store", None).await;
    assert_eq!(store["exposed"], true, "{store}");
    assert!(e.file(GONE).exists(), "an existing leftover is the person's data: never deleted on its own");
}

/// A file that doesn't open (a hashed name, a damaged file) still counts, as one.
#[tokio::test]
async fn a_file_that_does_not_open_counts_as_one() {
    let e = setup().await;
    std::fs::write(e.vault.join("h-00112233445566778899aabbccddeeff.bin"), b"not a vault file").unwrap();
    std::fs::write(e.vault.join("bitwarden.tmp-1"), b"ignored").unwrap();
    assert_eq!(e.local_count().await, 1);
    let bot = e.bots().await.remove(0);
    let (s, v) = e.bot_off(&bot).await;
    assert_refused(s, &v, 1);
}

#[tokio::test]
async fn deleting_a_team_deletes_its_saved_passwords() {
    let e = setup().await;
    let bot = e.bots().await.remove(0);
    let team = NewTeam {
        name: "t".into(),
        description: None,
        icon: None,
        lead_kwant_id: bot.clone(),
        member_kwants: vec![bot.clone()],
        workspace_path: None,
        repo_url: None,
    };
    let team = e.state.db.create_team(&SpaceId::default_space(), &team).await.unwrap().id;
    e.save_for(&team).await;
    e.save_for(&bot).await;
    assert_eq!(e.local_count().await, 2);

    let (s, v) = e.person("DELETE", &format!("/api/teams/{team}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.file(&team).exists(), "the deleted team's file is removed");
    assert!(e.file(&bot).exists(), "its lead bot's own passwords stay");
    assert_eq!(e.local_count().await, 1);
    let (s, v) = e.bot_off(&bot).await;
    assert_refused(s, &v, 1);
}

/// Channels go through the same clean-up, and an id that still exists is never wiped.
#[tokio::test]
async fn deleting_a_channel_deletes_its_saved_passwords_and_live_owners_are_kept() {
    let e = setup().await;
    let bot = e.bots().await.remove(0);
    let channel = NewChannel {
        name: "ops".into(),
        description: None,
        lead_kwant_id: bot.clone(),
        member_kwants: vec![bot.clone()],
        routing_mode: "auto".into(),
    };
    let channel = e.state.db.create_channel(&SpaceId::default_space(), &channel).await.unwrap().id;
    e.save_for(&channel).await;
    e.save_for(&bot).await;

    // Asked to forget owners that still exist: nothing happens
    forget_deleted_owners(&e.state, &[&channel, &bot]).await;
    assert!(e.file(&channel).exists() && e.file(&bot).exists());

    assert!(e.state.db.delete_empty_channel(&channel).await.unwrap());
    forget_deleted_owners(&e.state, &[&channel]).await;
    assert!(!e.file(&channel).exists(), "the deleted channel's file is removed");
    assert!(e.file(&bot).exists());
    assert_eq!(e.local_count().await, 1);
}

/// Spaces are archived, never deleted: their bots keep existing, and so does the count.
#[tokio::test]
async fn an_archived_space_keeps_counting() {
    let e = setup().await;
    let (s, v) = e.person("POST", "/api/spaces", Some(json!({"name": "Side"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let space = SpaceId::new(v["id"].as_str().unwrap());
    e.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let new_bot = NewKwant {
        name: "Sidey".into(),
        title: "t".into(),
        persona_prompt: "p".into(),
        soul: String::new(),
        avatar_sprite: "architect".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    let bot = e.state.db.create_kwant(&space, &new_bot).await.unwrap().id;
    e.save_for(&bot).await;
    e.state.db.archive_space(&space).await.unwrap();
    assert_eq!(e.local_count().await, 1);
    let other = e.bots().await.remove(0);
    let (s, v) = e.bot_off(&other).await;
    assert_refused(s, &v, 1);
}
