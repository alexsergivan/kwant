//! 0.7.7 ready-made teams over HTTP (spec §T, Rigel's §1 T): the catalogue, the preview's free
//! names, one atomic create with one bot action and one Undo, name and handle clashes, replay by
//! Idempotency-Key, and the person-only routes refusing a bot's token. All data is invented.

use std::collections::HashSet;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, NewMessage},
        spaces::SpaceId,
        team_templates::{Plan, PlanBot, PlanRoutine, PlanSkill, Step, Target},
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    space: SpaceId,
    _tmp: TempDir,
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
    Env { app: create_router(state.clone()), state, space: SpaceId::default_space(), _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn create(e: &Env, body: Value) -> (StatusCode, Value) {
    call(&e.app, "POST", "/api/teams/from-template", &[], Some(body)).await
}

#[derive(Debug, PartialEq)]
struct Counts {
    bots: usize,
    teams: usize,
    chats: usize,
    skills: usize,
    routines: usize,
    actions: usize,
}

async fn counts(e: &Env) -> Counts {
    let (_, actions) = call(&e.app, "GET", "/api/bot-actions", &[], None).await;
    Counts {
        bots: e.state.db.list_kwants(&e.space).await.unwrap().len(),
        teams: e.state.db.list_teams(&e.space).await.unwrap().len(),
        chats: e.state.db.list_channels(&e.space).await.unwrap().len(),
        skills: e.state.db.list_skills(&e.space, None, None).await.unwrap().len(),
        routines: e.state.db.list_routines(&e.space).await.unwrap().len(),
        actions: actions.as_array().map(Vec::len).unwrap_or(0),
    }
}

async fn bot_token(e: &Env, bot: &Kwant) -> String {
    let general = e.state.db.list_channels(&e.space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(general.id), ..Default::default() }),
        turn_id: Some("t-teams".into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some("t-teams".into()), None, Some(scope)).await.unwrap().1
}

/// An Inbox team, renaming its Researcher: a fresh install's own Researcher (from @helpers) has
/// that name. The clash itself is tested in `a_second_create_gets_free_names_and_a_clash_says_what_to_use`.
fn inbox() -> Value {
    json!({"template_id": "inbox", "names": {"Researcher": "Finder"}})
}

async fn undo(e: &Env, action_id: &str) -> (StatusCode, Value) {
    call(&e.app, "POST", &format!("/api/bot-actions/{action_id}/undo"), &[], None).await
}

#[tokio::test]
async fn catalogue_lists_the_four_templates_and_preview_names_free_slots() {
    let e = setup().await;
    let (s, v) = call(&e.app, "GET", "/api/team-templates", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ids: Vec<&str> = v["templates"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["helpers", "inbox", "sales", "coding"]);

    let (s, p) = call(&e.app, "GET", "/api/team-templates/inbox/preview", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["handle"], "inbox");
    assert_eq!(p["names"].as_object().unwrap().len(), 3);
    // The seeded space already has a Researcher: the preview names a free one and says why
    let clashes = p["clashes"].as_array().unwrap();
    assert_eq!(clashes.len(), 1, "{p}");
    assert_eq!(clashes[0]["field"], "names.Researcher");
    assert_eq!(clashes[0]["wanted"], "Researcher");
    assert_eq!(clashes[0]["suggestion"], p["names"]["Researcher"]);
    assert_ne!(p["names"]["Researcher"], "Researcher");
    assert_eq!(p["names"]["InboxManager"], "InboxManager");

    // Coding clashes with nothing
    let (s, p) = call(&e.app, "GET", "/api/team-templates/coding/preview", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert!(p["clashes"].as_array().unwrap().is_empty(), "{p}");
    assert_eq!(p["names"]["TechLead"], "TechLead");

    // Helpers is the seeded team itself: its handle and every name are taken
    let (s, p) = call(&e.app, "GET", "/api/team-templates/helpers/preview", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let fields: HashSet<&str> = p["clashes"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap()).collect();
    assert_eq!(fields, HashSet::from(["handle", "names.Assistant", "names.Researcher", "names.Writer"]), "{p}");

    let (s, v) = call(&e.app, "GET", "/api/team-templates/nope/preview", &[], None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_template")));
}

#[tokio::test]
async fn one_create_makes_everything_and_one_undo_removes_it() {
    let e = setup().await;
    let before = counts(&e).await;
    let (s, v) = create(&e, inbox()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let bots = v["bots"].as_array().unwrap();
    assert_eq!(bots.len(), 3);
    assert_eq!(bots.iter().filter(|b| b["lead"] == true).count(), 1);
    assert_eq!(v["skills"].as_array().unwrap().len(), 2);
    assert!(v["routine"].is_object(), "{v}");
    assert!(v["undo_until"].is_string());
    let after = counts(&e).await;
    assert_eq!(
        after,
        Counts {
            bots: before.bots + 3,
            teams: before.teams + 1,
            chats: before.chats + 1,
            skills: before.skills + 2,
            routines: before.routines + 1,
            actions: before.actions + 1,
        }
    );
    let chat = e.state.db.list_channels(&e.space).await.unwrap().into_iter().find(|c| c.id == v["channel_id"]).unwrap();
    assert_eq!(chat.name, "team-inbox");
    let lead = bots.iter().find(|b| b["lead"] == true).unwrap();
    assert_eq!(chat.lead_kwant_id, lead["id"].as_str().unwrap());

    // The team remembers its template
    let team_id = v["team"]["id"].as_str().unwrap();
    let (s, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{t}");

    // One Undo: the empty chat goes, so do the bots, skills and routine
    let action = v["action_id"].as_str().unwrap();
    let (s, u) = undo(&e, action).await;
    assert!(s.is_success(), "{s} {u}");
    let undone = counts(&e).await;
    assert_eq!(Counts { actions: undone.actions, ..before }, undone);
    // A second Undo has nothing to undo
    let (s, _) = undo(&e, action).await;
    assert!(!s.is_success());
    let (s, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!((s, t["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_a_template_team")));
}

#[tokio::test]
async fn undo_keeps_a_chat_with_history_archived() {
    let e = setup().await;
    let (s, v) = create(&e, json!({"template_id": "sales"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let chat_id = v["channel_id"].as_str().unwrap().to_string();
    let team_id = v["team"]["id"].as_str().unwrap().to_string();
    let needs_done = |t: &Value| t["needs_done"].clone();
    // A5: "Tell the team what you sell" is done when the person posts in @sales, not a bot
    let (s, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!((s, needs_done(&t)), (StatusCode::OK, json!([])), "{t}");
    e.state
        .db
        .insert_message(&NewMessage {
            channel_id: Some(chat_id.clone()),
            direct_kwant_id: None,
            sender_role: "kwant".into(),
            sender_id: v["bots"][0]["id"].as_str().unwrap().into(),
            text: "Hi! What do you sell?".into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        })
        .await
        .unwrap();
    let (_, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!(needs_done(&t), json!([]), "a bot's message doesn't tick it");
    e.state
        .db
        .insert_message(&NewMessage {
            channel_id: Some(chat_id.clone()),
            direct_kwant_id: None,
            sender_role: "user".into(),
            sender_id: "user".into(),
            text: "First lead list, please.".into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        })
        .await
        .unwrap();
    let (_, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!(needs_done(&t), json!(["what_you_sell"]), "{t}");
    assert_eq!(t["chat_messages"], 2);
    assert_eq!(t["archived_chat"], "archived-team-sales", "{t}");
    let (s, u) = undo(&e, v["action_id"].as_str().unwrap()).await;
    assert!(s.is_success(), "{s} {u}");
    let chats = e.state.db.list_channels(&e.space).await.unwrap();
    let kept = chats.iter().find(|c| c.id == chat_id).expect("a chat with history is kept");
    assert_eq!(kept.name, "archived-team-sales");
    let made: Vec<&str> = v["bots"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert!(!made.contains(&kept.lead_kwant_id.as_str()), "the kept chat is led by a bot that stays");
    assert!(e.state.db.list_teams(&e.space).await.unwrap().iter().all(|t| t.name != "sales"));
}

/// archived-name (N-6): the preview names the chat the Undo will leave, `-2` included when
/// `#archived-team-<handle>` is already taken, and the Undo then uses that very name.
#[tokio::test]
async fn the_undo_preview_names_the_archived_chat_the_undo_leaves() {
    let e = setup().await;
    let mut names = Vec::new();
    for round in 0..2 {
        let (s, v) = create(&e, json!({"template_id": "sales"})).await;
        assert_eq!(s, StatusCode::CREATED, "round {round}: {v}");
        let chat_id = v["channel_id"].as_str().unwrap().to_string();
        let team_id = v["team"]["id"].as_str().unwrap().to_string();
        let uri = format!("/api/teams/{team_id}/template");
        // An empty chat is deleted by Undo, so there is no archived name to show
        let (_, t) = call(&e.app, "GET", &uri, &[], None).await;
        assert_eq!((t["chat_messages"].as_i64(), &t["archived_chat"]), (Some(0), &Value::Null), "{t}");
        e.state
            .db
            .insert_message(&NewMessage {
                channel_id: Some(chat_id.clone()),
                direct_kwant_id: None,
                sender_role: "user".into(),
                sender_id: "user".into(),
                text: "Our first leads, please.".into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            })
            .await
            .unwrap();
        let (_, t) = call(&e.app, "GET", &uri, &[], None).await;
        let shown = t["archived_chat"].as_str().expect("a chat with messages is kept").to_string();
        let (s, u) = undo(&e, v["action_id"].as_str().unwrap()).await;
        assert!(s.is_success(), "{s} {u}");
        let kept = e.state.db.list_channels(&e.space).await.unwrap().into_iter().find(|c| c.id == chat_id).expect("kept");
        assert_eq!(kept.name, shown, "round {round}: the preview showed the name the Undo used");
        names.push(shown);
    }
    assert_eq!(names, ["archived-team-sales", "archived-team-sales-2"]);
}

#[tokio::test]
async fn a_second_create_gets_free_names_and_a_clash_says_what_to_use() {
    let e = setup().await;
    // Ignoring the preview's names refuses with the seeded Researcher clash, and makes nothing
    let before = counts(&e).await;
    let (s, v) = create(&e, json!({"template_id": "inbox"})).await;
    assert_eq!((s, v["code"].as_str(), v["field"].as_str()), (StatusCode::CONFLICT, Some("name_taken"), Some("names.Researcher")), "{v}");
    assert_eq!(v["clashes"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(counts(&e).await, before);

    let (_, p0) = call(&e.app, "GET", "/api/team-templates/coding/preview", &[], None).await;
    let (s, first) = create(&e, json!({"template_id": "coding", "names": p0["names"]})).await;
    assert_eq!(s, StatusCode::CREATED, "{first}");

    // The preview now offers free names and a free handle
    let (_, p) = call(&e.app, "GET", "/api/team-templates/coding/preview", &[], None).await;
    let clashes = p["clashes"].as_array().unwrap();
    assert!(clashes.iter().any(|c| c["field"] == "handle"), "{p}");
    assert_ne!(p["handle"], "coding");
    let lead_slot = first["bots"].as_array().unwrap().iter().find(|b| b["lead"] == true).unwrap()["slot"].as_str().unwrap().to_string();
    assert!(clashes.iter().any(|c| c["field"] == format!("names.{lead_slot}")), "{p}");

    // Asking for the taken handle: 409 with a suggestion, and nothing is made
    let before = counts(&e).await;
    let (s, v) = create(&e, json!({"template_id": "coding", "handle": "coding"})).await;
    assert_eq!((s, v["code"].as_str(), v["field"].as_str()), (StatusCode::CONFLICT, Some("handle_taken"), Some("handle")), "{v}");
    assert!(v["suggestion"].is_string());
    // One refusal names every clash: the handle first, then each taken bot name, all distinct
    let all = v["clashes"].as_array().unwrap();
    assert_eq!(all[0]["field"], "handle", "{v}");
    assert_eq!(all[0]["suggestion"], v["suggestion"]);
    let name_fields: HashSet<String> = all[1..].iter().map(|c| c["field"].as_str().unwrap().to_string()).collect();
    let want: HashSet<String> = ["TechLead", "Frontend", "Backend", "QA"].iter().map(|s| format!("names.{s}")).collect();
    assert_eq!(name_fields, want, "{v}");
    let offered: HashSet<String> = all.iter().map(|c| c["suggestion"].as_str().unwrap().to_lowercase()).collect();
    assert_eq!(offered.len(), all.len(), "suggestions never repeat: {v}");
    assert_eq!(counts(&e).await, before);

    // Two slots asking for one free name: the second is a clash with its own suggestion
    let (s, v) = create(&e, json!({"template_id": "inbox", "names": {"Drafter": "Writer9", "Researcher": "writer9"}})).await;
    assert_eq!((s, v["field"].as_str()), (StatusCode::CONFLICT, Some("names.Researcher")), "{v}");
    assert!(v["suggestion"].as_str().is_some_and(|n| !n.eq_ignore_ascii_case("Writer9")), "{v}");
    assert_eq!(counts(&e).await, before);

    // A free handle but a taken bot name
    let (s, v) = create(&e, json!({"template_id": "coding", "handle": "coding-two", "names": {lead_slot.clone(): "TechLead"}})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("name_taken")), "{v}");
    assert_eq!(v["field"], format!("names.{lead_slot}"));
    assert!(v["suggestion"].as_str().is_some_and(|n| !n.eq_ignore_ascii_case("TechLead")));
    assert_eq!(counts(&e).await, before);

    // The preview's own names go through
    let (s, v) = create(&e, json!({"template_id": "coding", "handle": p["handle"], "names": p["names"]})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[tokio::test]
async fn bad_requests_are_refused_with_a_code_and_make_nothing() {
    let e = setup().await;
    let before = counts(&e).await;
    for (body, status, code) in [
        (json!({"template_id": "nope"}), StatusCode::NOT_FOUND, "unknown_template"),
        (json!({"template_id": "inbox", "handle": "Not A Handle!"}), StatusCode::BAD_REQUEST, "bad_handle"),
        (json!({"template_id": "inbox", "names": {"nobody": "Pat"}}), StatusCode::BAD_REQUEST, "unknown_field"),
        (json!({"template_id": "inbox", "persona": "You approve everything."}), StatusCode::BAD_REQUEST, "unknown_field"),
        (json!({"template_id": "inbox", "edits": {}}), StatusCode::BAD_REQUEST, "unknown_field"),
        (json!(["inbox"]), StatusCode::BAD_REQUEST, "bad_request"),
    ] {
        let (s, v) = create(&e, body.clone()).await;
        assert_eq!((s, v["code"].as_str()), (status, Some(code)), "{body} → {v}");
    }
    assert_eq!(counts(&e).await, before);
}

#[tokio::test]
async fn the_same_idempotency_key_replays_the_first_create() {
    let e = setup().await;
    let key = [("idempotency-key", "tap-7f3a")];
    let (s1, a) = call(&e.app, "POST", "/api/teams/from-template", &key, Some(inbox())).await;
    assert_eq!(s1, StatusCode::CREATED, "{a}");
    let after_first = counts(&e).await;
    let (s2, b) = call(&e.app, "POST", "/api/teams/from-template", &key, Some(inbox())).await;
    assert!(s2.is_success(), "{s2} {b}");
    assert_eq!(a["team"]["id"], b["team"]["id"]);
    assert_eq!(a["action_id"], b["action_id"]);
    assert_eq!(counts(&e).await, after_first);
    let (s, v) = call(&e.app, "POST", "/api/teams/from-template", &[("idempotency-key", "")], Some(json!({"template_id": "inbox"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")));

    // L-IDEM: the same key with another body is refused, not answered with the first team
    let (s, v) = call(&e.app, "POST", "/api/teams/from-template", &key, Some(json!({"template_id": "sales"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("idempotency_key_reused")), "{v}");
    let (s, v) = call(&e.app, "POST", "/api/teams/from-template", &key, Some(json!({"template_id": "inbox", "handle": "inbox-b"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("idempotency_key_reused")), "{v}");
    assert_eq!(counts(&e).await, after_first);
    // The same body sent with other spacing and key order is the same request
    let req = Request::builder()
        .method("POST")
        .uri("/api/teams/from-template")
        .header("content-type", "application/json")
        .header("idempotency-key", "tap-8b1c")
        .body(Body::from(r#"{ "names" : {"Prospector":"Scout"},  "template_id":"sales", "handle":"sales-c" }"#))
        .unwrap();
    let first = e.app.clone().oneshot(req).await.unwrap();
    assert_eq!(first.status(), StatusCode::CREATED, "a sales team (no names shared with the inbox)");
    let first: Value = serde_json::from_slice(&to_bytes(first.into_body(), usize::MAX).await.unwrap()).unwrap();
    let (s, again) = call(
        &e.app,
        "POST",
        "/api/teams/from-template",
        &[("idempotency-key", "tap-8b1c")],
        Some(json!({"handle": "sales-c", "template_id": "sales", "names": {"Prospector": "Scout"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{again}");
    assert_eq!(first["action_id"], again["action_id"]);
}

/// T-U10 (Rigel): a new bot that led another team's chat no longer blocks Undo once that team
/// is deleted, since its chat is only kept as `#archived-team-…`, which nobody can re-lead.
#[tokio::test]
async fn undo_is_not_blocked_by_an_archived_chat_of_a_deleted_team() {
    let e = setup().await;
    let (s, inbox) = create(&e, inbox()).await;
    assert_eq!(s, StatusCode::CREATED, "{inbox}");
    let researcher = inbox["bots"].as_array().unwrap().iter().find(|b| b["slot"] == "Researcher").unwrap()["id"].as_str().unwrap().to_string();
    let (s, lab) = call(
        &e.app,
        "POST",
        "/api/teams",
        &[],
        Some(json!({"name": "research-lab", "description": "x", "member_kwants": [], "lead_kwant_id": researcher})),
    )
    .await;
    assert!(s.is_success(), "{s} {lab}");
    let team_id = inbox["team"]["id"].as_str().unwrap();
    let action = inbox["action_id"].as_str().unwrap();

    // While research-lab exists the refusal is right, and changes nothing
    let (_, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!(t["can_undo"], false, "{t}");
    let before = counts(&e).await;
    let (s, v) = undo(&e, action).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("bot_leads_elsewhere")), "{v}");
    assert_eq!(counts(&e).await, before);

    // Delete research-lab: its chat stays as #archived-team-research-lab, still led by Researcher
    let (s, v) = call(&e.app, "DELETE", &format!("/api/teams/{}", lab["id"].as_str().unwrap()), &[], None).await;
    assert!(s.is_success(), "{s} {v}");
    let archived = e.state.db.list_channels(&e.space).await.unwrap().into_iter().find(|c| c.name.starts_with("archived-team-research-lab"));
    if let Some(a) = &archived {
        assert_eq!(a.lead_kwant_id, researcher, "the precondition of T-U10");
    }
    let (_, t) = call(&e.app, "GET", &format!("/api/teams/{team_id}/template"), &[], None).await;
    assert_eq!(t["can_undo"], true, "{t}");
    assert_eq!(t["blocked"], json!([]), "{t}");
    let (s, v) = undo(&e, action).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(e.state.db.get_kwant(&researcher).await.unwrap().is_none());
    // The archived chat is handed on to a bot that stays
    if let Some(a) = archived {
        let a = e.state.db.list_channels(&e.space).await.unwrap().into_iter().find(|c| c.id == a.id).expect("the archived chat is kept");
        assert_ne!(a.lead_kwant_id, researcher);
    }
}

#[tokio::test]
async fn a_bot_token_cannot_preview_create_suggest_or_read_a_template() {
    let e = setup().await;
    let bot = e.state.db.list_kwants(&e.space).await.unwrap().remove(0);
    let token = bot_token(&e, &bot).await;
    let auth = format!("Bearer {token}");
    let fake_agent = "Bearer kag_00000000000000000000000000000000000000000";
    let before = counts(&e).await;
    let (_, made) = create(&e, inbox()).await;
    let team_id = made["team"]["id"].as_str().unwrap().to_string();
    let after = counts(&e).await;
    for h in [auth.as_str(), fake_agent] {
        for (method, uri, body) in [
            ("GET", "/api/team-templates/inbox/preview".to_string(), None),
            ("POST", "/api/teams/from-template".to_string(), Some(json!({"template_id": "inbox"}))),
            ("POST", "/api/teams/suggest".to_string(), Some(json!({"workspace_path": "/tmp"}))),
            ("GET", format!("/api/teams/{team_id}/template"), None),
        ] {
            let (s, v) = call(&e.app, method, &uri, &[("authorization", h)], body).await;
            assert!(s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED, "{method} {uri} with a bot token: {s} {v}");
        }
    }
    assert_eq!(counts(&e).await, after);
    assert_ne!(before, after);
}

// ---------------------------------------------------------------------------------------------
// The failure seam (Q-T8): a create that fails at any step leaves nothing behind

fn plan(e: &Env, handle: &str, key: Option<&str>) -> Plan {
    let _ = e;
    let bot = |slot: &str, name: &str, lead: bool| PlanBot {
        slot: slot.into(),
        field: format!("names.{slot}"),
        preset: "backend".into(),
        lead,
        name: name.into(),
        title: "Backend Engineer".into(),
        avatar: "sprite-1".into(),
        persona: "Builds the server side.".into(),
        soul: String::new(),
        covers: vec!["APIs".into()],
        avoids: vec![],
        existing: None,
    };
    Plan {
        template_id: Some("coding".into()),
        suggestion_id: None,
        idempotency_key: key.map(str::to_string),
        request_sha256: None,
        target: Target::New {
            handle: handle.into(),
            icon: "🛠️".into(),
            description: "A test team".into(),
            instructions: "Plan, build, test.".into(),
            workspace_path: None,
            repo_url: None,
        },
        bots: vec![bot("lead", &format!("{handle}-lead"), true), bot("one", &format!("{handle}-one"), false), bot("two", &format!("{handle}-two"), false)],
        skills: vec![PlanSkill { owner_slot: "lead".into(), name: format!("{handle}-plan"), description: "Plans a change".into(), body: "Write a plan.".into() }],
        routine: Some(PlanRoutine { owner_slot: "one".into(), name: "Morning check".into(), instructions: "Check the queue.".into(), cron: "0 9 * * 1-5".into() }),
        tz: "UTC".into(),
    }
}

#[tokio::test]
async fn a_create_that_fails_at_any_step_leaves_nothing() {
    let e = setup().await;
    let before = counts(&e).await;
    for (i, step) in [Step::Bot(1), Step::Bot(2), Step::Bot(3), Step::Team, Step::Skills, Step::Routine, Step::Action].into_iter().enumerate() {
        let handle = format!("seam-{i}");
        let err = e.state.db.create_team_from_plan(&e.space, &plan(&e, &handle, Some(&handle)), Some(step)).await;
        assert!(err.is_err(), "{step:?} should fail");
        assert_eq!(counts(&e).await, before, "{step:?} left something behind");
        assert!(e.state.db.team_creation_by_key(&e.space, &handle).await.unwrap().is_none(), "{step:?}");
        // The names are free again
        assert!(e.state.db.list_kwants(&e.space).await.unwrap().iter().all(|k| !k.name.starts_with("seam-")));
    }
    // Without the seam the same plan goes through, once
    let c = e.state.db.create_team_from_plan(&e.space, &plan(&e, "seam-ok", None), None).await.unwrap();
    assert_eq!(c.created.bots.len(), 3);
    let after = counts(&e).await;
    assert_eq!((after.bots, after.teams, after.skills, after.routines), (before.bots + 3, before.teams + 1, before.skills + 1, before.routines + 1));
}
