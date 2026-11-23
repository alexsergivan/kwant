//! pa4c-server A: the person edits a quiz draft's questions on the main port (the Draft edit
//! API, autonomy/pocket-apps/pa4c-server/impl.md). An edited or added question becomes the
//! person's own ("yours", not source-checked), the questions they didn't touch keep their check
//! with no new fetch, and the dry run runs again. A bad edit changes nothing; an open or closed
//! quiz refuses; another space's draft is 404; with the switch off the routes are gone; a bot
//! can't call them, and a bot's later update can't drop the person's question.

mod quizmaster_support;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Path, State},
    http::{Request, StatusCode},
    response::Html,
    routing::get,
    Router,
};
use quizmaster_support::{card, ready_friday_quiz, setup, space, start_pages, tool, turn, Env, PAGES};
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tower::ServiceExt;

/// A call as the person, from this machine.
async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    person_on(&e.app, method, uri, body).await
}

async fn person_on(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The same call with a bot's turn token.
async fn as_bot(e: &Env, token: &str, method: &str, uri: &str) -> StatusCode {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("authorization", format!("Bearer {token}"));
    let mut req = req.body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    e.app.clone().oneshot(req).await.unwrap().status()
}

fn statuses(draft: &Value) -> Vec<String> {
    draft["questions"].as_array().unwrap().iter().map(|q| q["status"].as_str().unwrap().to_string()).collect()
}

fn fetches(pages: &quizmaster_support::Pages) -> usize {
    pages.hits.lock().unwrap().values().sum()
}

async fn ready(e: &Env) -> (String, String, quizmaster_support::Pages) {
    let (pages_url, pages) = start_pages().await;
    let token = turn(e, &e.bot, "t-pa4c").await;
    let (id, _) = ready_friday_quiz(e, &token, &pages_url).await;
    (id, token, pages)
}

#[tokio::test]
async fn reading_a_draft_lists_every_question_with_its_check() {
    let e = setup(true).await;
    let (id, _, _) = ready(&e).await;
    let (status, body) = person(&e, "GET", &format!("/api/quiz-drafts/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let d = &body["draft"];
    assert_eq!(d["id"], json!(id));
    assert_eq!(d["state"], json!("ready"));
    assert_eq!(d["header"], json!("4 questions · 4 checked · 0 yours"));
    assert_eq!(d["counts"], json!({"questions": 4, "checked": 4, "yours": 0, "not_checked": 0}));
    assert_eq!(statuses(d), vec!["checked"; 4]);
    let q2 = &d["questions"][1];
    assert_eq!((q2["n"].as_u64(), q2["round"].as_u64(), q2["kind"].as_str()), (Some(2), Some(1), Some("text")));
    assert_eq!(q2["question"], json!("Which footballer won the Ballon d'Or in 1998?"));
    assert_eq!(q2["answer"], json!("Zinedine Zidane"));
    assert_eq!(q2["accepted"], json!(["Zidane", "Zizou"]));
    assert!(d["questions"][3]["picture"].is_object(), "the photo question shows its picture");
    assert!(d["dry_run"]["ok"].as_bool().unwrap(), "{d}");
    assert!(d["updated_at"].is_i64());
    assert_eq!(body["card"]["state"], json!("ready"), "the same card the bot shows");
}

#[tokio::test]
async fn editing_a_checked_question_makes_it_yours_and_reruns_the_dry_run() {
    let e = setup(true).await;
    let (id, _, pages) = ready(&e).await;
    let before = fetches(&pages);
    let (_, read) = person(&e, "GET", &format!("/api/quiz-drafts/{id}"), None).await;
    let base = read["draft"]["updated_at"].clone();

    let (status, body) = person(
        &e,
        "PATCH",
        &format!("/api/quiz-drafts/{id}/questions/2"),
        Some(json!({"answer": "Zinedine Zidane", "accepted": ["Zidane", "Zizou", "ZZ"], "base": base})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let d = &body["draft"];
    assert_eq!(statuses(d), vec!["checked", "yours", "checked", "checked"], "only the edited question changes");
    assert_eq!(d["header"], json!("4 questions · 3 checked · 1 yours"));
    assert_eq!(d["state"], json!("ready"), "your own question still lets the quiz open: {d}");
    assert_eq!(d["questions"][1]["accepted"], json!(["Zidane", "Zizou", "ZZ"]));
    assert!(d["questions"][1].get("source_url").is_none_or(Value::is_null), "no source is claimed for your own question");
    assert!(d["dry_run"]["ok"].as_bool().unwrap(), "the dry run ran again");
    assert_eq!(fetches(&pages), before, "questions you didn't touch keep their check: nothing is fetched again");
    assert_eq!(body["card"]["yours"], json!(1), "the card agrees: {}", body["card"]);

    // Stored: the spec has the new spelling and no source on that question
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    let spec: Value = serde_json::from_str(&row.spec).unwrap();
    assert_eq!(spec["rounds"][0]["questions"][1]["accept"], json!(["Zidane", "Zizou", "ZZ"]));
    assert_eq!(spec["rounds"][0]["questions"][1]["source"], json!({"own": true}));
    assert_eq!(row.state, "ready");

    // A stale base is refused and changes nothing
    let (status, body) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/1"), Some(json!({"question": "Which band made Definitely Maybe?", "base": base}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!("changed"));
}

#[tokio::test]
async fn adding_and_deleting_questions() {
    let e = setup(true).await;
    let (id, _, pages) = ready(&e).await;
    let before = fetches(&pages);
    let (status, body) = person(&e, "POST", &format!("/api/quiz-drafts/{id}/questions"), Some(json!({"round": 1, "question": "Which fuzzy brown fruit is green inside?", "answer": "Kiwi", "accepted": ["kiwifruit", "kiwi fruit"]}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let d = &body["draft"];
    assert_eq!(d["header"], json!("5 questions · 4 checked · 1 yours"));
    assert_eq!(statuses(d), vec!["checked", "checked", "yours", "checked", "checked"], "added at the end of round 1");
    assert_eq!(d["questions"][2]["kind"], json!("text"));
    assert_eq!(d["state"], json!("ready"));
    assert_eq!(fetches(&pages), before);

    let (status, body) = person(&e, "DELETE", &format!("/api/quiz-drafts/{id}/questions/1"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let d = &body["draft"];
    assert_eq!(statuses(d), vec!["checked", "yours", "checked", "checked"], "the rest keep their check after the move");
    assert_eq!(d["questions"][0]["question"], json!("Which footballer won the Ballon d'Or in 1998?"));
    assert_eq!(d["state"], json!("ready"));

    // A question that isn't there
    let (status, _) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/9"), Some(json!({"answer": "x"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_bad_edit_returns_numbered_problems_and_changes_nothing() {
    let e = setup(true).await;
    let (id, _, _) = ready(&e).await;
    let before = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();

    let (status, body) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/1"), Some(json!({"answer": "Radiohead", "question": ""}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], json!("invalid"));
    let problems: Vec<&str> = body["problems"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    assert!(problems.len() >= 2, "every problem at once: {problems:?}");
    assert!(problems[0].starts_with("1. ") && problems[1].starts_with("2. "), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("Radiohead")), "{problems:?}");

    let after = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert_eq!(after, before, "nothing was saved");

    // A body that isn't an edit
    let (status, _) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/1"), Some(json!({"answer": {"x": 1}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap(), before);
}

#[tokio::test]
async fn open_and_closed_quizzes_refuse_and_other_spaces_are_404() {
    let e = setup(true).await;
    let (id, _, _) = ready(&e).await;
    let mut row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();

    for (st, words) in [("open", "An open quiz's questions can't change."), ("closed", "A closed quiz's questions can't change.")] {
        row.state = st.into();
        e.state.db.quiz_draft_save(&row).await.unwrap();
        let (status, body) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/2"), Some(json!({"answer": "Zizou"}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{st}: {body}");
        assert_eq!(body["message"], json!(words));
        let (status, _) = person(&e, "DELETE", &format!("/api/quiz-drafts/{id}/questions/2"), None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = person(&e, "GET", &format!("/api/quiz-drafts/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "an open quiz can still be read");
    }

    // The same draft filed under another space
    let mut other = row.clone();
    other.id = "qd_otherspace1".into();
    other.space_id = "space-not-yours".into();
    other.state = "ready".into();
    e.state.db.quiz_draft_save(&other).await.unwrap();
    let (status, body) = person(&e, "GET", "/api/quiz-drafts/qd_otherspace1", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _) = person(&e, "PATCH", "/api/quiz-drafts/qd_otherspace1/questions/1", Some(json!({"answer": "Oasis"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = person(&e, "GET", "/api/quiz-drafts/qd_nothere", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_switch_off_hides_the_routes_and_bots_never_call_them() {
    let e = setup(true).await;
    let (id, token, _) = ready(&e).await;
    assert_eq!(as_bot(&e, &token, "GET", &format!("/api/quiz-drafts/{id}")).await, StatusCode::FORBIDDEN, "person only");
    assert_eq!(as_bot(&e, &token, "DELETE", &format!("/api/quiz-drafts/{id}/questions/1")).await, StatusCode::FORBIDDEN);

    e.state.db.set_setting(kwnt::server::quizmaster_tools::SETTING, "off").await.unwrap();
    let (status, body) = person(&e, "GET", &format!("/api/quiz-drafts/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("pocket_apps_off"));
    let (status, _) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/2"), Some(json!({"answer": "Zizou"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_bot_update_keeps_the_persons_question() {
    let e = setup(true).await;
    let (id, token, _) = ready(&e).await;
    let old_spec: Value = serde_json::from_str(&e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap().spec).unwrap();
    let (status, _) = person(&e, "PATCH", &format!("/api/quiz-drafts/{id}/questions/2"), Some(json!({"accepted": ["Zidane", "Zizou", "ZZ"]}))).await;
    assert_eq!(status, StatusCode::OK);

    // The bot hands in its old spec: the person's question would be lost, so nothing changes
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "spec": old_spec})).await;
    assert!(err, "{text}");
    assert!(text.contains("round 1, question 2"), "{text}");
    assert!(text.contains("\"ZZ\""), "the message carries the person's question to copy: {text}");
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert!(row.spec.contains("\"ZZ\""), "the person's edit is still there");

    // With the person's question kept as it is, the update goes through and it stays theirs
    let stored: Value = serde_json::from_str(&row.spec).unwrap();
    let mut spec = old_spec.clone();
    spec["rounds"][0]["questions"][1] = stored["rounds"][0]["questions"][1].clone();
    spec["title"] = json!("Friday Pub Quiz, take two");
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "spec": spec})).await;
    assert!(!err, "{text}");
    let c = card(&text);
    assert_eq!((c["state"].as_str(), c["yours"].as_u64()), (Some("ready"), Some(1)), "{text}");
}

/// A source server whose first page fetch is where the person saves an edit: the bot's update is
/// still checking its sources when the person's PATCH lands.
#[derive(Clone)]
struct Race {
    app: Router,
    uri: String,
    fired: Arc<AtomicBool>,
    person_status: Arc<Mutex<Option<StatusCode>>>,
}

async fn race_page(State(r): State<Race>, Path(slug): Path<String>) -> Result<Html<String>, StatusCode> {
    if !r.fired.swap(true, Ordering::SeqCst) {
        let (status, _) = person_on(&r.app, "PATCH", &r.uri, Some(json!({"accepted": ["Zidane", "Zizou", "ZZ"]}))).await;
        *r.person_status.lock().unwrap() = Some(status);
    }
    let body = PAGES.iter().find(|(s, _)| *s == slug).map(|(_, t)| *t).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Html(format!("<html><head><title>{slug}</title></head><body><p>{body}</p></body></html>")))
}

#[tokio::test]
async fn a_persons_edit_during_a_bot_update_wins() {
    let e = setup(true).await;
    let (id, token, _) = ready(&e).await;
    let before = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    let race = Race {
        app: e.app.clone(),
        uri: format!("/api/quiz-drafts/{id}/questions/2"),
        fired: Arc::new(AtomicBool::new(false)),
        person_status: Arc::new(Mutex::new(None)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let race_url = format!("http://{}/wiki", listener.local_addr().unwrap());
    let server = Router::new().route("/wiki/:slug", get(race_page)).with_state(race.clone());
    tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });

    // The bot moves question 1 to a new page, so its update fetches; the person edits question 2 meanwhile
    let mut spec: Value = serde_json::from_str(&before.spec).unwrap();
    spec["rounds"][0]["questions"][0]["source"]["url"] = json!(format!("{race_url}/definitely-maybe"));
    spec["title"] = json!("Friday Pub Quiz, the bot's take");
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "spec": spec})).await;
    assert_eq!(*race.person_status.lock().unwrap(), Some(StatusCode::OK), "the person's edit landed mid-update");
    assert!(err, "{text}");
    assert!(text.contains("The person changed this quiz while you were working"), "{text}");

    // The person's edit is what's stored; none of the bot's update is
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert!(row.spec.contains("\"ZZ\""), "the person's edit is still there");
    assert!(!row.spec.contains(&race_url), "the bot's stale spec didn't overwrite it");
    assert_eq!(row.title, before.title);
    assert_ne!(row.own_keys, "[]", "question 2 is the person's");

    // Calling again, the bot is told which question to keep
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "spec": spec})).await;
    assert!(err, "{text}");
    assert!(text.contains("round 1, question 2") && text.contains("\"ZZ\""), "{text}");
}
