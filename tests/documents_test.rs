//! Camera-to-Completed (`documents`): the person-only letters API, validation, the one-at-a-time
//! reader, a text PDF plus a photo read end to end by a stub model, field corrections, deadline
//! reminders, delete, and that the letter's words never reach the logs or a notification.
//! No real model is called: every test sets `state.documents` to a stub reader.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{NaiveDate, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    documents::{
        extract::Desk,
        reader::{LetterReader, Page},
        reminders,
    },
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const PDF: &[u8] = include_bytes!("fixtures/letters/water-bill.pdf");
const PNG: &[u8] = include_bytes!("fixtures/letters/letter-page2.png");

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup_with(reader: Arc<dyn LetterReader>) -> Env {
    Logs::global();
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.documents = Arc::new(Desk::with_reader(reader));
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

/// The model's answer for the water bill: the PDF is page 1, the photo (a reply slip) page 2.
fn water_bill_answer() -> Value {
    json!({
        "transcript": "Reply slip. Tick the box and return this slip by post.",
        "sender": {"value": "Northfield Water Ltd", "quote": "Northfield Water Ltd", "uncertain": false},
        "request": {"value": "Pay the overdue water bill", "quote": "Your water bill of GBP 142.60 is overdue.", "uncertain": false},
        "reply_to": {"value": "billing@northfield-water.example", "quote": "billing@northfield-water.example", "uncertain": false},
        "deadline": {"value": "2026-11-30", "quote": "Please pay the full amount by 30 November 2026.", "uncertain": false},
        "references": [{"value": "NW-448812", "quote": "Account number: NW-448812", "uncertain": false}],
        "amounts": [{"value": "GBP 142.60", "quote": "GBP 142.60", "uncertain": false}],
        "checklist": [
            {"value": "Return the reply slip", "quote": "Tick the box and return this slip by post.", "uncertain": false},
            {"value": "Call the helpline", "quote": "Ring us on 0800 000 000.", "uncertain": false}
        ]
    })
}

/// Answers with a fixed text, counts its calls, records what it was sent, and can be held.
struct Stub {
    answer: Result<String, String>,
    calls: AtomicUsize,
    pages: Mutex<Vec<String>>,
    prompts: Mutex<Vec<String>>,
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl Stub {
    fn new(answer: Result<String, String>) -> Arc<Self> {
        Arc::new(Stub { answer, calls: AtomicUsize::new(0), pages: Mutex::default(), prompts: Mutex::default(), gate: None })
    }
    fn gated(answer: &str, gate: Arc<tokio::sync::Semaphore>) -> Arc<Self> {
        Arc::new(Stub { answer: Ok(answer.into()), calls: AtomicUsize::new(0), pages: Mutex::default(), prompts: Mutex::default(), gate: Some(gate) })
    }
}

#[async_trait::async_trait]
impl LetterReader for Stub {
    fn name(&self) -> String {
        "stub".into()
    }
    async fn read(&self, _system: &str, prompt: &str, pages: &[Page]) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.pages.lock().unwrap().extend(pages.iter().map(|p| p.mime.clone()));
        self.prompts.lock().unwrap().push(prompt.to_string());
        if let Some(g) = &self.gate {
            let _ = g.acquire().await.unwrap();
        }
        self.answer.clone()
    }
}

async fn call(app: &Router, method: &str, uri: &str, bearer: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Env {
    async fn person(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        call(&self.app, method, uri, Some(self.state.api_token.reveal()), body).await
    }

    async fn turn_token(&self, folder: &Path) -> String {
        let bot = self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
        let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
        mcp_runtime::prepare_turn_with_self(&self.state, &bot, None, Some(folder.to_path_buf()), Some(scope)).await.unwrap().1
    }

    async fn upload(&self, name: &str, bytes: &[u8]) -> String {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/attachments?name={name}"))
            .header("authorization", format!("Bearer {}", self.state.api_token.reveal()))
            .header("content-type", "application/octet-stream")
            .body(Body::from(bytes.to_vec()))
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
        v["id"].as_str().unwrap().to_string()
    }

    async fn create(&self, ids: &[&str]) -> (StatusCode, Value) {
        self.person("POST", "/api/documents", Some(json!({"attachment_ids": ids}))).await
    }

    /// Polls the letter until it is no longer being read.
    async fn read_through(&self, case_id: &str) -> Value {
        for _ in 0..200 {
            let (s, v) = self.person("GET", &format!("/api/documents/{case_id}"), None).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            if v["extraction"]["status"] != "running" {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the letter was never read");
    }
}

/// A unique PNG each call, so two uploads never share a blob.
fn png(n: u8) -> Vec<u8> {
    let mut b = PNG.to_vec();
    b.push(n);
    b
}

/// Collects everything logged while it is the thread's default subscriber.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    /// Everything every test in this file logs (a global subscriber: tasks the server spawns log
    /// on other threads and callsites cache their interest).
    fn global() -> Logs {
        static LOGS: std::sync::OnceLock<Logs> = std::sync::OnceLock::new();
        LOGS.get_or_init(|| {
            let logs = Logs::default();
            let sink = logs.clone();
            let subscriber = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_ansi(false).with_writer(move || sink.clone()).finish();
            tracing::subscriber::set_global_default(subscriber).unwrap();
            logs
        })
        .clone()
    }
}

#[tokio::test]
async fn every_route_is_the_persons_only() {
    let e = setup_with(Stub::new(Ok("{}".into()))).await;
    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;
    let id = e.upload("p.png", &png(1)).await;
    let routes: Vec<(&str, &str, Option<Value>)> = vec![
        ("POST", "/api/documents", Some(json!({"attachment_ids": [id]}))),
        ("GET", "/api/documents/x", None),
        ("DELETE", "/api/documents/x", None),
        ("PATCH", "/api/zombie/cases/x/fields", Some(json!({"deadline": {}}))),
    ];
    for (method, uri, body) in &routes {
        let (s, v) = call(&e.app, method, uri, None, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
        let (s, v) = call(&e.app, method, uri, Some("not-the-token"), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a wrong token: {v}");
    }
    assert!(e.state.db.zombie_cases(None).await.unwrap().is_empty(), "nothing was stored by a refused call");

    // A letter's pages are the person's too, even through the attachments API
    let (s, v) = e.create(&[&id]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let case_id = v["case_id"].as_str().unwrap().to_string();
    e.read_through(&case_id).await;
    for uri in [format!("/api/attachments/{id}"), format!("/api/attachments/{id}/content"), format!("/api/attachments?context_id=document:{case_id}")] {
        let (s, _) = call(&e.app, "GET", &uri, Some(&turn), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{uri} with a bot's turn token");
        let (s, _) = call(&e.app, "GET", &uri, None, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri} without a token");
        let (s, _) = e.person("GET", &uri, None).await;
        assert_eq!(s, StatusCode::OK, "{uri} as the person");
    }
}

#[tokio::test]
async fn bad_requests_are_refused_before_anything_is_stored() {
    let e = setup_with(Stub::new(Ok("{}".into()))).await;
    let ids: Vec<String> = {
        let mut v = Vec::new();
        for n in 0..6 {
            v.push(e.upload(&format!("p{n}.png"), &png(10 + n)).await);
        }
        v
    };
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let text = e.upload("notes.txt", b"just some notes, not a letter").await;
    for (body, want) in [
        (json!({"attachment_ids": []}), StatusCode::BAD_REQUEST),
        (json!({"attachment_ids": refs}), StatusCode::BAD_REQUEST),
        (json!({"attachment_ids": [refs[0], refs[0]]}), StatusCode::BAD_REQUEST),
        (json!({"attachment_ids": ["not-a-uuid"]}), StatusCode::BAD_REQUEST),
        (json!({"attachment_ids": [uuid::Uuid::new_v4().to_string()]}), StatusCode::BAD_REQUEST),
        (json!({"attachment_ids": [text]}), StatusCode::BAD_REQUEST),
        (json!({"pages": [refs[0]]}), StatusCode::BAD_REQUEST),
    ] {
        let (s, v) = e.person("POST", "/api/documents", Some(body.clone())).await;
        assert_eq!(s, want, "{body}: {v}");
    }
    assert!(e.state.db.zombie_cases(None).await.unwrap().is_empty());

    // An upload already used for one letter can't start another
    let (s, v) = e.create(&[refs[0]]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    e.read_through(v["case_id"].as_str().unwrap()).await;
    let (s, v) = e.create(&[refs[0]]).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    // Unknown letters and cases that aren't letters
    let (s, _) = e.person("GET", "/api/documents/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("DELETE", "/api/documents/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("PATCH", "/api/zombie/cases/nope/fields", Some(json!({}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn one_letter_is_read_at_a_time() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let stub = Stub::gated("{}", gate.clone());
    let e = setup_with(stub.clone()).await;
    let first = e.upload("a.png", &png(1)).await;
    let second = e.upload("b.png", &png(2)).await;
    let (s, v) = e.create(&[&first]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let case_id = v["case_id"].as_str().unwrap().to_string();

    let (s, v) = e.create(&[&second]).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "busy");
    // The refused upload is still free to use
    assert!(e.state.db.get_attachment(&second).await.unwrap().unwrap().message_id.is_none());

    // While it is being read it can't be corrected or deleted
    let (s, _) = e.person("PATCH", &format!("/api/zombie/cases/{case_id}/fields"), Some(json!({"sender": {"value": "X"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = e.person("DELETE", &format!("/api/documents/{case_id}"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);

    gate.add_permits(10);
    e.read_through(&case_id).await;
    let (s, v) = e.create(&[&second]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "free again once the first is read: {v}");
}

#[tokio::test]
async fn a_photo_no_model_can_read_is_left_for_the_person() {
    let e = setup_with(Stub::new(Err("the claude command could not start".into()))).await;
    let id = e.upload("photo.png", &png(3)).await;
    let (_, v) = e.create(&[&id]).await;
    let doc = e.read_through(v["case_id"].as_str().unwrap()).await;
    let x = &doc["extraction"];
    assert_eq!(x["status"], "unreadable", "{doc}");
    assert!(x["note"].as_str().unwrap().contains("yourself"), "{doc}");
    assert!(x["fields"]["sender"].is_null());
    assert_eq!(x["text"], "");
    assert!(doc["reminders"].as_array().unwrap().is_empty());

    // The person fills it in; a deadline they type is confirmed and schedules reminders
    let case_id = doc["case"]["id"].as_str().unwrap();
    let (s, v) = e
        .person(
            "PATCH",
            &format!("/api/zombie/cases/{case_id}/fields"),
            Some(json!({"sender": {"value": "City Council"}, "deadline": {"value": "2099-03-01"}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["case"]["title"], "Letter from City Council");
    assert_eq!(v["reminders"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_text_pdf_and_a_photo_are_read_in_one_model_call() {
    let logs = Logs::global();

    let stub = Stub::new(Ok(water_bill_answer().to_string()));
    let e = setup_with(stub.clone()).await;
    let pdf = e.upload("water-bill.pdf", PDF).await;
    let photo = e.upload("slip.png", &png(4)).await;
    let (s, v) = e.create(&[&pdf, &photo]).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let case_id = v["case_id"].as_str().unwrap().to_string();

    // The case shows in Loose ends at once
    let (_, list) = e.person("GET", "/api/zombie/cases", None).await;
    assert!(list["cases"].as_array().unwrap().iter().any(|c| c["id"] == case_id.as_str() && c["kind"] == "document_request"), "{list}");

    let doc = e.read_through(&case_id).await;
    let x = &doc["extraction"];
    assert_eq!(x["status"], "read", "{doc}");
    assert_eq!(x["method"], "pdf_text+model_vision");
    assert_eq!(x["read_by"], "stub");
    assert_eq!(x["pages"].as_array().unwrap().len(), 2);
    assert_eq!(x["pages"][0]["url"], format!("/api/attachments/{pdf}/content"));

    // One call; only the photo went as an image, the PDF's text went in the prompt
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*stub.pages.lock().unwrap(), vec!["image/png".to_string()]);
    assert!(stub.prompts.lock().unwrap()[0].contains("NW-448812"));

    let f = &x["fields"];
    assert_eq!(f["sender"]["value"], "Northfield Water Ltd");
    assert_eq!(f["sender"]["uncertain"], false);
    assert_eq!(f["deadline"]["value"], "2026-11-30");
    assert_eq!(f["deadline"]["confirmed"], false, "nothing is confirmed until the person says so");
    assert_eq!(f["amounts"][0]["value"], "GBP 142.60");
    assert_eq!(f["references"][0]["value"], "NW-448812");
    assert_eq!(f["checklist"][0]["uncertain"], false, "a quote from the photo's transcript stands");
    assert_eq!(f["checklist"][0]["done"], false);
    assert_eq!(f["checklist"][1]["uncertain"], true, "a quote that is in neither page is uncertain");
    assert!(x["text"].as_str().unwrap().contains("Reply slip"));

    let case = &doc["case"];
    assert_eq!(case["title"], "Letter from Northfield Water Ltd");
    assert!(case["deadline"].is_null(), "an unconfirmed deadline is not the case's deadline");
    assert!(doc["reminders"].as_array().unwrap().is_empty(), "an unconfirmed deadline schedules nothing");
    assert_eq!(case["draft"]["channel"], "email");
    assert_eq!(case["draft"]["to"], "billing@northfield-water.example");

    // Unsure about the deadline: still nothing scheduled
    let fields = format!("/api/zombie/cases/{case_id}/fields");
    let (s, v) = e.person("PATCH", &fields, Some(json!({"deadline": {"value": "30/11/2026"}}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "only ISO dates: {v}");
    let (s, v) = e.person("PATCH", &fields, Some(json!({"deadline": null}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["reminders"].as_array().unwrap().is_empty());

    // The person sets and so confirms a deadline: three reminders, 7 days, 1 day and the day
    let (s, v) = e.person("PATCH", &fields, Some(json!({"deadline": {"value": "2099-03-01"}, "checklist": [{"value": "Return the reply slip", "done": true}]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["extraction"]["fields"]["deadline"]["confirmed"], true);
    assert_eq!(v["extraction"]["fields"]["checklist"].as_array().unwrap().len(), 1);
    assert_eq!(v["extraction"]["fields"]["checklist"][0]["done"], true);
    assert_eq!(v["extraction"]["fields"]["checklist"][0]["quote"], "Tick the box and return this slip by post.", "an unchanged item keeps its quote");
    assert_eq!(v["case"]["deadline"].as_str().unwrap().get(..10), Some("2099-03-01"));
    let rem = v["reminders"].as_array().unwrap();
    let offsets: Vec<i64> = rem.iter().map(|r| r["offset_days"].as_i64().unwrap()).collect();
    assert_eq!(offsets.len(), 3);
    for o in [7, 1, 0] {
        assert!(offsets.contains(&o), "{offsets:?}");
    }

    // The tick: the 7-day reminder fires with counts only
    let mut events = e.state.event_sender.subscribe();
    let due7: chrono::DateTime<Utc> = rem.iter().find(|r| r["offset_days"] == 7).unwrap()["due_at"].as_str().unwrap().parse().unwrap();
    assert!(reminders::run_once(&e.state, due7 - chrono::Duration::minutes(1)).await.is_empty(), "not yet");
    let fired = reminders::run_once(&e.state, due7).await;
    assert_eq!(fired.total(), 1);
    let mut seen = None;
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::Notify { title, body, kind, .. } = ev {
            seen = Some((kind, title, body));
        }
    }
    let (kind, title, body) = seen.expect("a notification");
    assert_eq!(kind, "loose_ends");
    assert_eq!(title, "Letter deadline: 1 to deal with");
    assert_eq!(body, "1 letter has a deadline in 7 days.");
    assert!(reminders::run_once(&e.state, due7).await.is_empty(), "a reminder fires once");

    // Done clears the reminders; reopening brings back the ones still ahead
    let (s, v) = e.person("POST", &format!("/api/zombie/cases/{case_id}/done"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(e.state.db.document_reminders(&case_id).await.unwrap().is_empty());
    let (s, v) = e.person("POST", &format!("/api/zombie/cases/{case_id}/reopen"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let back = e.state.db.document_reminders(&case_id).await.unwrap();
    assert!(!back.is_empty() && back.len() <= 3, "{back:?}");

    // Delete: the case, the extracted text, the reminders, the pages and their files
    let root = kwnt::server::attachments::root(&e.state);
    let sha = e.state.db.get_attachment(&pdf).await.unwrap().unwrap().sha256;
    let blob = kwnt::server::attachments::blob_path(&root, &sha).unwrap();
    assert!(blob.exists());
    let (s, v) = e.person("DELETE", &format!("/api/documents/{case_id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(e.state.db.zombie_case(&case_id).await.unwrap().is_none());
    assert!(e.state.db.document_extraction(&case_id).await.unwrap().is_none());
    assert!(e.state.db.document_reminders(&case_id).await.unwrap().is_empty());
    assert!(e.state.db.get_attachment(&pdf).await.unwrap().is_none());
    assert!(e.state.db.get_attachment(&photo).await.unwrap().is_none());
    assert!(!blob.exists(), "the page file is gone");
    let (s, _) = e.person("GET", &format!("/api/documents/{case_id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Nothing the letter says was logged
    let logged = logs.text();
    assert!(logged.contains("documents: reading a letter"), "the capture works");
    for words in ["NW-448812", "Northfield", "142.60", "overdue", "Reply slip", "northfield-water", "City Council"] {
        assert!(!logged.contains(words), "'{words}' reached the logs");
    }
}

#[test]
fn the_reminder_schedule_follows_the_injected_clock() {
    let deadline = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
    let early = "2026-10-20T12:00:00Z".parse().unwrap();
    assert_eq!(reminders::schedule(deadline, "UTC", early).len(), 3);
    let late = "2026-11-29T10:00:00Z".parse().unwrap();
    let s = reminders::schedule(deadline, "UTC", late);
    assert_eq!(s.iter().map(|(o, _)| *o).collect::<Vec<_>>(), vec![1, 0]);
    assert_eq!(s[0].1, late, "the reminder due earlier today fires now");
}
