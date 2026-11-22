//! 32u: a suggested Coding team (the Coding template's roster, for a project with nothing to read)
//! says its bots' and team's saved names in every text its bots read, as a ready-made team does
//! since 32p. All folders are temporary and all names are invented.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    teams::templates,
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
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
    let config = Config { data_dir: tmp.path().join("data"), ..Config::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    Env { app: create_router(state.clone()), state, space: SpaceId::default_space(), _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A project with only a README: nothing points to a role, so kwant suggests the Coding
/// template's roster (TechLead, Frontend, Backend, QA).
fn bare_project(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("README.md"), "Read me").unwrap();
    root
}

async fn fallback_suggestion(e: &Env, folder: &Path) -> Value {
    let (s, sug) = call(&e.app, "POST", "/api/teams/suggest", Some(json!({ "workspace_path": folder.to_str().unwrap() }))).await;
    assert_eq!(s, StatusCode::OK, "{sug}");
    assert_eq!(sug["fallback"], true, "{sug}");
    let keys: Vec<&str> = sug["bots"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["lead", "frontend", "backend", "qa"], "{sug}");
    sug
}

/// Posted for the Coding template, so the team also gets its starter skills.
async fn create(e: &Env, sug: &Value, edits: Value) -> Value {
    let (s, made) = call(&e.app, "POST", "/api/teams/from-template", Some(json!({ "suggestion_id": sug["suggestion_id"], "template_id": "coding", "edits": edits }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    made
}

/// `word` standing alone in `text`: not part of a longer name such as `Pixel2` or `@coding-two`.
fn says(text: &str, word: &str) -> bool {
    let wordy = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word)
        .any(|(i, _)| !text[..i].chars().next_back().is_some_and(wordy) && !text[i + word.len()..].chars().next().is_some_and(|c| wordy(c) || c == '-'))
}

/// Every persona and soul of the bots the create made or used, and their skills (description and body), by label.
async fn bot_texts(e: &Env, made: &Value) -> Vec<(String, String)> {
    let ids: Vec<String> = made["bots"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap().to_string()).collect();
    let mut out = Vec::new();
    for k in e.state.db.list_kwants(&e.space).await.unwrap().into_iter().filter(|k| ids.contains(&k.id)) {
        out.push((format!("{} persona", k.name), k.persona_prompt));
        out.push((format!("{} soul", k.name), k.soul));
    }
    for s in e.state.db.list_skills(&e.space, None, None).await.unwrap() {
        if s.kwant_id.as_ref().is_some_and(|k| ids.contains(k)) {
            out.push((format!("skill {} description", s.name), s.description));
            out.push((format!("skill {}", s.name), s.body));
        }
    }
    out
}

fn text<'a>(texts: &'a [(String, String)], label: &str) -> &'a str {
    &texts.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label} in {texts:?}")).1
}

async fn team_instructions(e: &Env, made: &Value) -> String {
    let id = made["team"]["id"].as_str().unwrap();
    e.state.db.list_teams(&e.space).await.unwrap().into_iter().find(|t| t.id == id).unwrap().instructions.unwrap_or_default()
}

fn coding() -> &'static templates::Template {
    templates::get("coding").unwrap()
}

#[tokio::test]
async fn a_suggested_coding_team_with_frontend_renamed_pixel_on_web_says_so() {
    let e = setup().await;
    let tmp = tempdir().unwrap();
    let sug = fallback_suggestion(&e, &bare_project(tmp.path(), "shop")).await;
    let made = create(&e, &sug, json!({ "handle": "web", "rows": { "frontend": { "name": "Pixel" } } })).await;
    assert_eq!(made["team"]["name"], "web", "{made}");
    let texts = bot_texts(&e, &made).await;
    assert_eq!(texts.len(), 4 * 2 + 2 * 2, "4 personas, 4 souls, 2 skills x2: {texts:?}");

    // Pixel's own persona names its team as saved
    let pixel = text(&texts, "Pixel persona");
    assert!(says(pixel, "@web"), "{pixel}");
    // The lead's persona and its plan skill hand screen work to Pixel
    assert!(says(text(&texts, "TechLead persona"), "Pixel"), "{}", text(&texts, "TechLead persona"));
    assert!(says(text(&texts, "skill Plan a change"), "Pixel"), "{}", text(&texts, "skill Plan a change"));
    for (label, t) in &texts {
        assert!(!says(t, "Frontend"), "{label} still names the template's Frontend: {t}");
        assert!(!says(t, "@coding"), "{label} still names the template's @coding: {t}");
    }
    // Bots that kept their names keep their mentions; only the handle moved in their texts
    let t = coding();
    let backend = t.bot("Backend").unwrap();
    assert_eq!(text(&texts, "Backend persona"), backend.persona.replace("@coding", "@web"));
    assert!(says(text(&texts, "TechLead persona"), "Backend") && says(text(&texts, "TechLead persona"), "QA"));
    assert!(says(text(&texts, "skill Test a change before review"), "TechLead"));
    for b in &t.bots {
        let name = if b.name == "Frontend" { "Pixel" } else { b.name.as_str() };
        assert_eq!(text(&texts, &format!("{name} soul")), b.soul, "{name}'s soul names no bot, so it is the template's");
    }
    let test_skill = t.skills.iter().find(|s| s.name == "Test a change before review").unwrap();
    assert_eq!(text(&texts, "skill Test a change before review"), test_skill.body, "names nobody renamed");
}

#[tokio::test]
async fn a_suggested_coding_team_left_as_is_keeps_every_template_text() {
    let e = setup().await;
    let tmp = tempdir().unwrap();
    // A folder named like the template's own handle, so the suggestion's handle is the template's
    let sug = fallback_suggestion(&e, &bare_project(tmp.path(), "coding")).await;
    assert_eq!(sug["handle"], "coding", "{sug}");
    let made = create(&e, &sug, json!({})).await;
    let texts = bot_texts(&e, &made).await;
    let t = coding();
    for b in &t.bots {
        assert_eq!(text(&texts, &format!("{} persona", b.name)), b.persona, "{}", b.name);
        assert_eq!(text(&texts, &format!("{} soul", b.name)), b.soul, "{}", b.name);
    }
    for s in &t.skills {
        assert_eq!(text(&texts, &format!("skill {} description", s.name)), s.description, "{}", s.name);
        assert_eq!(text(&texts, &format!("skill {}", s.name)), s.body, "{}", s.name);
    }
}

#[tokio::test]
async fn a_row_filled_by_an_existing_bot_is_named_by_that_bot() {
    let e = setup().await;
    let tmp = tempdir().unwrap();
    // The space's own Researcher takes the QA row: the others' texts send testing to Researcher
    let researcher = e.state.db.list_kwants(&e.space).await.unwrap().into_iter().find(|k| k.name == "Researcher").unwrap();
    let researcher_persona = researcher.persona_prompt.clone();
    let sug = fallback_suggestion(&e, &bare_project(tmp.path(), "shop")).await;
    let made = create(&e, &sug, json!({ "handle": "web", "rows": { "qa": { "use_existing": researcher.id } } })).await;
    let texts = bot_texts(&e, &made).await;

    let lead = text(&texts, "TechLead persona");
    assert!(says(lead, "Researcher") && !says(lead, "QA"), "{lead}");
    let plan = text(&texts, "skill Plan a change");
    assert!(says(plan, "Researcher") && !says(plan, "QA"), "{plan}");
    // The team's rules ("QA reads every pull request") name it too
    let rules = team_instructions(&e, &made).await;
    assert!(says(&rules, "Researcher") && !says(&rules, "QA"), "{rules}");
    // The reused bot keeps its own persona, and gets no starter skill
    assert_eq!(text(&texts, "Researcher persona"), researcher_persona);
    assert!(texts.iter().all(|(l, _)| l != "skill Test a change before review"), "{texts:?}");
}
