//! Zombie Mission Hunter (`zombie`): the rules against a synthetic mailbox (precision and recall
//! are printed), the case store's transitions over repeated scans, the model double-check, the
//! person-only API, and a GitHub source against a local mock.
//!
//! `cargo test --test zombie_test -- --nocapture` shows the precision report.

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    extract::Query,
    http::{HeaderMap, Request, StatusCode},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, zombie::CaseRow, Database},
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
    zombie::{
        classify::Classifier,
        detect::{detect, Rules},
        hunt::{scan_with, ScanReport},
        imap::{conversations_from_raw, RawMail},
        settings::Settings,
        ConvKind, Conversation, MissionSource, Msg, SourceKind,
    },
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};
use kwnt::sandbox::credentials::CredentialStore;
use tower::ServiceExt;

// ---- the synthetic mailbox ----

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zombie_mailbox")
}

/// Every message, with `{{DATE:n}}` turned into the date `n` days before `now`.
fn load_mailbox(now: DateTime<Utc>) -> Vec<RawMail> {
    let mut out = Vec::new();
    for folder in ["inbox", "sent"] {
        let mut files: Vec<_> = std::fs::read_dir(fixture_dir().join(folder)).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let start = text.find("{{DATE:").expect("a date placeholder");
            let end = start + text[start..].find("}}").unwrap();
            let days: i64 = text[start + 7..end].parse().unwrap();
            let date = (now - Duration::days(days)).to_rfc2822();
            let text = format!("{}{}{}", &text[..start], date, &text[end + 2..]);
            out.push(RawMail { bytes: text.into_bytes(), in_sent: folder == "sent", received: None });
        }
    }
    out
}

/// `(subject, expected kind, note, holdout)`. Holdout threads were written after the rules were
/// tuned on the rest, so their score is the honest one.
type Expected = (String, Option<String>, String, bool);

fn manifest() -> (Vec<String>, Vec<Expected>) {
    let m: Value = serde_json::from_str(&std::fs::read_to_string(fixture_dir().join("manifest.json")).unwrap()).unwrap();
    let me = m["me"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let threads = m["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (t["subject"].as_str().unwrap().to_string(), t["expect"].as_str().map(String::from), t["note"].as_str().unwrap().to_string(), t["holdout"].as_bool().unwrap_or(false))
        })
        .collect();
    (me, threads)
}

fn root_subject(s: &str) -> String {
    let mut s = s.trim();
    while let Some(rest) = s.strip_prefix("Re: ").or_else(|| s.strip_prefix("RE: ")) {
        s = rest.trim();
    }
    s.to_string()
}

#[test]
fn synthetic_mailbox_precision() {
    let now = Utc::now();
    let (me, expected) = manifest();
    assert!(expected.len() >= 20, "the mailbox needs at least 20 threads");
    let convs = conversations_from_raw("src-fixture", &load_mailbox(now), &me);
    let rules = Rules { now, stale_days: 14, kwant_stale_days: 3 };
    let found: HashMap<String, Option<String>> = convs.iter().map(|c| (root_subject(&c.subject), detect(c, &rules).map(|k| k.kind.as_str().to_string()))).collect();

    assert_eq!(found.len(), expected.len(), "every thread once, nothing split or merged");
    let mut report = Vec::new();
    // [tuning, holdout] x (true+, false+, missed, wrong kind)
    let mut score = [(0, 0, 0, 0); 2];
    for (subject, expect, note, holdout) in &expected {
        let got = found.get(subject).unwrap_or_else(|| panic!("thread {subject:?} was not threaded"));
        let s = &mut score[usize::from(*holdout)];
        let mark = match (expect, got) {
            (Some(e), Some(g)) if e == g => {
                s.0 += 1;
                "ok"
            }
            (Some(_), Some(_)) => {
                s.0 += 1;
                s.3 += 1;
                "KIND"
            }
            (None, None) => "ok",
            (None, Some(_)) => {
                s.1 += 1;
                "FALSE+"
            }
            (Some(_), None) => {
                s.2 += 1;
                "MISSED"
            }
        };
        let set = if *holdout { "holdout" } else { "tuning" };
        report.push(format!("  {mark:6} {set:7} {subject:40} expected {:15} got {:15} ({note})", expect.as_deref().unwrap_or("-"), got.as_deref().unwrap_or("-")));
    }
    println!("Zombie Hunter rules on {} synthetic threads:\n{}", expected.len(), report.join("\n"));
    let total = (score[0].0 + score[1].0, score[0].1 + score[1].1, score[0].2 + score[1].2, score[0].3 + score[1].3);
    let pr = |(tp, fp, fn_, _): (i32, i32, i32, i32)| (tp as f64 / (tp + fp).max(1) as f64, tp as f64 / (tp + fn_).max(1) as f64);
    for (name, s) in [("tuning", score[0]), ("holdout", score[1]), ("all", total)] {
        let (p, r) = pr(s);
        println!("{name:8} precision {p:.2} ({}/{}), recall {r:.2} ({}/{}), wrong kind {}", s.0, s.0 + s.1, s.0, s.0 + s.2, s.3);
    }
    let (precision, recall) = pr(total);
    // The queue must not fill with noise: a false alarm costs the person's trust
    assert!(precision >= 0.9, "precision {precision:.2}");
    assert!(recall >= 0.7, "recall {recall:.2}");
}

#[test]
fn no_message_text_ends_up_in_a_case_title_or_why() {
    let now = Utc::now();
    let (me, _) = manifest();
    for c in conversations_from_raw("src-fixture", &load_mailbox(now), &me) {
        if let Some(k) = detect(&c, &Rules { now, stale_days: 14, kwant_stale_days: 3 }) {
            // The why is generated, and quotes come only from the conversation itself
            for m in &c.messages {
                if m.text.len() > 30 {
                    assert!(!k.why.contains(&m.text), "{}", k.why);
                }
            }
            for e in &k.evidence {
                assert!(c.texts().any(|t| kwnt::zombie::quote_in(t, &e.quote)), "evidence {:?} is not verbatim", e.quote);
            }
        }
    }
}

// ---- the case store over repeated scans ----

/// A source that returns whatever the test put in it.
#[derive(Clone, Default)]
struct FakeSource {
    convs: Arc<Mutex<Vec<Conversation>>>,
    fail: Arc<Mutex<bool>>,
}

impl FakeSource {
    fn set(&self, c: Vec<Conversation>) {
        *self.convs.lock().unwrap() = c;
    }
}

#[async_trait]
impl MissionSource for FakeSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Imap
    }
    fn id(&self) -> &str {
        "src-fake"
    }
    async fn conversations(&self, _since: DateTime<Utc>) -> Result<Vec<Conversation>, String> {
        if *self.fail.lock().unwrap() {
            return Err("the server could not be reached".into());
        }
        Ok(self.convs.lock().unwrap().clone())
    }
}

fn msg(from: &str, outbound: bool, at: DateTime<Utc>, text: &str) -> Msg {
    Msg { from: from.into(), outbound, at, text: text.into(), auto: false, bulk: false, message_id: None }
}

/// The person asked, `days` ago, and nobody answered.
fn asked(key: &str, days: i64, now: DateTime<Utc>) -> Conversation {
    let mut c = Conversation::new(SourceKind::Imap, "src-fake", key, ConvKind::Email, &format!("Question {key}"));
    c.counterpart = Some(format!("{key}@corp.example"));
    c.messages.push(msg("me@example.org", true, now - Duration::days(days), "Could you tell me the status of claim No. 4471-B?"));
    c
}

async fn db() -> (Database, TempDir) {
    let tmp = tempdir().unwrap();
    (Database::init(&tmp.path().join("data")).await.unwrap(), tmp)
}

async fn run(db: &Database, src: &FakeSource, classifier: Option<&dyn Classifier>, now: DateTime<Utc>) -> ScanReport {
    let sources: Vec<Box<dyn MissionSource>> = vec![Box::new(src.clone())];
    let settings = Settings { kwant_source: false, ..Default::default() };
    scan_with(db, &sources, classifier, &settings, now).await.unwrap()
}

async fn case(db: &Database, key: &str) -> CaseRow {
    db.zombie_case_by_key(&format!("imap:src-fake:{key}")).await.unwrap().expect("a case")
}

#[tokio::test]
async fn a_case_opens_once_and_dedupes() {
    let (db, _t) = db().await;
    let src = FakeSource::default();
    let now = Utc::now();
    src.set(vec![asked("a", 20, now), asked("fresh", 2, now)]);
    let r = run(&db, &src, None, now).await;
    assert_eq!((r.flagged, r.opened, r.open_total), (1, 1, 1), "{r:?}");
    let c = case(&db, "a").await;
    assert_eq!((c.status.as_str(), c.kind.as_str()), ("open", "awaiting_reply"));
    assert_eq!(c.reference.as_deref(), Some("4471-B"));
    let draft: Value = serde_json::from_str(c.draft.as_deref().unwrap()).unwrap();
    assert!(draft["body"].as_str().unwrap().contains("4471-B"), "{draft}");
    assert!(draft["mailto"].as_str().unwrap().starts_with("mailto:a%40corp.example"), "{draft}");

    let r = run(&db, &src, None, now + Duration::hours(1)).await;
    assert_eq!((r.opened, r.open_total), (0, 1), "a second scan finds the same case, not a new one");
    assert_eq!(case(&db, "a").await.id, c.id);
}

#[tokio::test]
async fn dismissed_stays_dismissed_and_answered_closes() {
    let (db, _t) = db().await;
    let src = FakeSource::default();
    let now = Utc::now();
    src.set(vec![asked("a", 20, now), asked("b", 20, now)]);
    run(&db, &src, None, now).await;
    let mut a = case(&db, "a").await;
    a.status = "dismissed".into();
    db.zombie_put_case(&a).await.unwrap();
    run(&db, &src, None, now + Duration::days(1)).await;
    assert_eq!(case(&db, "a").await.status, "dismissed", "a dismissal is remembered");

    // b gets an answer
    let mut b = asked("b", 20, now);
    b.messages.push(msg("b@corp.example", false, now - Duration::days(1), "All sorted, it was approved."));
    src.set(vec![asked("a", 20, now), b]);
    let r = run(&db, &src, None, now + Duration::days(1)).await;
    let b = case(&db, "b").await;
    assert_eq!((b.status.as_str(), b.resolution.as_deref()), ("closed", Some("resolved")), "{r:?}");
}

#[tokio::test]
async fn a_follow_up_waits_then_reopens_if_unanswered() {
    let (db, _t) = db().await;
    let src = FakeSource::default();
    let now = Utc::now();
    src.set(vec![asked("a", 20, now)]);
    run(&db, &src, None, now).await;
    let mut a = case(&db, "a").await;
    a.status = "waiting_reply".into();
    a.waiting_since = Some(now);
    db.zombie_put_case(&a).await.unwrap();

    // The person's follow-up shows up in the thread; still within the wait
    let mut followed = asked("a", 20, now);
    followed.messages.push(msg("me@example.org", true, now, "Following up on claim 4471-B: any news?"));
    src.set(vec![followed.clone()]);
    run(&db, &src, None, now + Duration::days(5)).await;
    assert_eq!(case(&db, "a").await.status, "waiting_reply");

    // Two weeks on, no answer: back in the queue, counted
    let r = run(&db, &src, None, now + Duration::days(15)).await;
    let a = case(&db, "a").await;
    assert_eq!((a.status.as_str(), a.follow_ups), ("open", 1), "{r:?}");
    assert_eq!(r.reopened, 1);

    // The answer finally comes
    let mut a2 = a;
    a2.status = "waiting_reply".into();
    a2.waiting_since = Some(now + Duration::days(15));
    db.zombie_put_case(&a2).await.unwrap();
    followed.messages.push(msg("a@corp.example", false, now + Duration::days(16), "Approved, the money is on its way."));
    src.set(vec![followed]);
    run(&db, &src, None, now + Duration::days(17)).await;
    assert_eq!(case(&db, "a").await.status, "closed");
}

#[tokio::test]
async fn a_snooze_ends_on_its_date() {
    let (db, _t) = db().await;
    let src = FakeSource::default();
    let now = Utc::now();
    src.set(vec![asked("a", 20, now)]);
    run(&db, &src, None, now).await;
    let mut a = case(&db, "a").await;
    a.status = "snoozed".into();
    a.snoozed_until = Some(now + Duration::days(7));
    db.zombie_put_case(&a).await.unwrap();
    run(&db, &src, None, now + Duration::days(3)).await;
    assert_eq!(case(&db, "a").await.status, "snoozed");
    run(&db, &src, None, now + Duration::days(8)).await;
    assert_eq!(case(&db, "a").await.status, "open");
}

#[tokio::test]
async fn a_muted_sender_and_a_failing_source_open_nothing() {
    let (db, _t) = db().await;
    let src = FakeSource::default();
    let now = Utc::now();
    db.zombie_mute("src-fake", "sender", "A@corp.example").await.unwrap();
    src.set(vec![asked("a", 20, now)]);
    let r = run(&db, &src, None, now).await;
    assert_eq!(r.opened, 0);

    *src.fail.lock().unwrap() = true;
    let r = run(&db, &src, None, now).await;
    assert!(!r.sources[0].ok);
    assert_eq!(r.sources[0].error.as_deref(), Some("the server could not be reached"));
}

// ---- the model double-check ----

/// Answers every candidate the same way, and records what it was shown.
struct FakeModel {
    answer: String,
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl Classifier for FakeModel {
    fn name(&self) -> String {
        "fake".into()
    }
    async fn ask(&self, _system: &str, prompt: &str) -> Result<String, String> {
        self.seen.lock().unwrap().push(prompt.into());
        Ok(self.answer.clone())
    }
}

#[tokio::test]
async fn the_model_must_quote_or_the_case_is_dropped() {
    let now = Utc::now();
    for (answer, opens) in [
        (r#"{"keep": true, "quote": "the status of claim No. 4471-B"}"#, 1),
        // A quote that isn't in the thread: an invention, dropped
        (r#"{"keep": true, "quote": "please call me back tomorrow"}"#, 0),
        (r#"{"keep": false, "quote": ""}"#, 0),
        // Unreadable: the rules' finding stands, flagged unchecked
        ("I think so", 1),
    ] {
        let (db, _t) = db().await;
        let src = FakeSource::default();
        src.set(vec![asked("a", 20, now)]);
        let model = FakeModel { answer: answer.into(), seen: Mutex::new(Vec::new()) };
        let r = run(&db, &src, Some(&model), now).await;
        assert_eq!(r.opened, opens, "{answer}: {r:?}");
        assert_eq!(r.dropped_by_model, 1 - opens, "{answer}");
        let seen = model.seen.lock().unwrap();
        assert!(seen[0].contains("4471-B"), "the model sees the conversation");
    }
}

// ---- the API ----

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    Env { app: create_router(state.clone()), state, _tmp: tmp }
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

/// `call` from another machine (a non-loopback Host), without a token.
async fn call_remote(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").header("host", "kwnt.example.com");
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
    /// One open case, straight from a scan.
    async fn seed(&self, key: &str) -> String {
        let src = FakeSource::default();
        let now = Utc::now();
        let mut c = asked(key, 20, now);
        c.counterpart = Some("claims@insurer.example".into());
        src.set(vec![c]);
        run(&self.state.db, &src, None, now).await;
        case(&self.state.db, key).await.id
    }
}

#[tokio::test]
async fn every_route_is_the_persons_only() {
    let e = setup().await;
    let folder = tempdir().unwrap();
    let turn = e.turn_token(folder.path()).await;
    let routes: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/api/zombie/cases", None),
        ("GET", "/api/zombie/cases/x", None),
        ("POST", "/api/zombie/cases/x/snooze", Some(json!({"days": 7}))),
        ("POST", "/api/zombie/cases/x/dismiss", Some(json!({}))),
        ("POST", "/api/zombie/cases/x/done", None),
        ("POST", "/api/zombie/cases/x/followed-up", None),
        ("POST", "/api/zombie/cases/x/reopen", None),
        ("POST", "/api/zombie/cases/x/draft", None),
        ("GET", "/api/zombie/sources", None),
        ("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "x", "config": {}, "secret": "DUMMY"}))),
        ("PATCH", "/api/zombie/sources/x", Some(json!({"label": "y"}))),
        ("DELETE", "/api/zombie/sources/x", None),
        ("POST", "/api/zombie/sources/x/test", None),
        ("POST", "/api/zombie/run", None),
        ("GET", "/api/zombie/status", None),
        ("GET", "/api/zombie/settings", None),
        ("PUT", "/api/zombie/settings", Some(serde_json::to_value(Settings::default()).unwrap())),
        ("GET", "/api/zombie/reviews", None),
    ];
    for (method, uri, body) in &routes {
        let (s, v) = call_remote(&e.app, method, uri, body.clone()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} from another machine without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
    }
    let (s, v) = e.person("GET", "/api/zombie/cases", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["cases"], json!([]));
}

#[tokio::test]
async fn the_person_works_a_case() {
    let e = setup().await;
    let id = e.seed("a").await;
    let (s, v) = e.person("GET", "/api/zombie/cases?status=open", None).await;
    assert_eq!(s, StatusCode::OK);
    let c = &v["cases"][0];
    for field in ["id", "status", "kind", "conv_kind", "source", "title", "why", "evidence", "draft", "age_days", "last_activity_at", "follow_ups"] {
        assert!(c.get(field).is_some(), "{field} missing in {c}");
    }
    assert_eq!(c["reference"], "4471-B");
    assert!(c["evidence"][0]["quote"].as_str().unwrap().contains("4471-B"));
    assert!(c["draft"]["mailto"].as_str().unwrap().starts_with("mailto:claims%40insurer.example"));

    let (s, _) = e.person("POST", &format!("/api/zombie/cases/{id}/snooze"), Some(json!({"days": 0}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = e.person("POST", &format!("/api/zombie/cases/{id}/snooze"), Some(json!({"days": 7}))).await;
    assert_eq!((s, v["status"].as_str()), (StatusCode::OK, Some("snoozed")));
    let (_, v) = e.person("POST", &format!("/api/zombie/cases/{id}/followed-up"), None).await;
    assert_eq!(v["status"], "waiting_reply");
    assert!(v["waiting_since"].is_string());
    let (_, v) = e.person("POST", &format!("/api/zombie/cases/{id}/done"), None).await;
    assert_eq!((v["status"].as_str(), v["resolution"].as_str()), (Some("closed"), Some("done")));
    let (_, v) = e.person("POST", &format!("/api/zombie/cases/{id}/reopen"), None).await;
    assert_eq!(v["status"], "open");

    let (s, _) = e.person("POST", &format!("/api/zombie/cases/{id}/dismiss"), Some(json!({"scope": "everyone"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = e.person("POST", &format!("/api/zombie/cases/{id}/dismiss"), Some(json!({"scope": "sender", "reason": "not mine"}))).await;
    assert_eq!((v["status"].as_str(), v["dismiss_reason"].as_str()), (Some("dismissed"), Some("not mine")));
    // The sender is muted: another of their threads doesn't open
    let src = FakeSource::default();
    let mut other = asked("b", 20, Utc::now());
    other.counterpart = Some("claims@insurer.example".into());
    src.set(vec![other]);
    assert_eq!(run(&e.state.db, &src, None, Utc::now()).await.opened, 0);
    // Reopening unmutes
    e.person("POST", &format!("/api/zombie/cases/{id}/reopen"), None).await;
    assert_eq!(run(&e.state.db, &src, None, Utc::now()).await.opened, 1);

    let (s, _) = e.person("GET", "/api/zombie/cases/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e.person("GET", "/api/zombie/cases?status=weird", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_are_validated() {
    let e = setup().await;
    let (_, v) = e.person("GET", "/api/zombie/settings", None).await;
    assert_eq!((v["stale_days"].as_i64(), v["review_time"].as_str(), v["max_per_review"].as_i64()), (Some(14), Some("09:00"), Some(5)));
    for bad in [json!({"stale_days": 0}), json!({"review_time": "25:00"}), json!({"review_weekdays": ["funday"]}), json!({"llm": "maybe"}), json!({"retention_days": 0}), json!({"max_per_review": 0})] {
        let (s, v) = e.person("PUT", "/api/zombie/settings", Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} was accepted: {v}");
    }
    let (s, v) = e.person("PUT", "/api/zombie/settings", Some(json!({"stale_days": 10, "review_weekdays": ["mon", "thu"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = e.person("GET", "/api/zombie/status", None).await;
    assert!(v["next_review_at"].is_string(), "{v}");
    let (_, v) = e.person("GET", "/api/zombie/sources", None).await;
    assert_eq!(v["sources"][0]["kind"], "kwant");
    assert_eq!(v["sources"][0]["builtin"], true);
}

// ---- GitHub against a local mock ----

const MOCK_TOKEN: &str = "ghp_FIXTURE_ONLY_not_a_real_token";

async fn github_mock() -> (String, Arc<Mutex<Vec<String>>>) {
    let hits: Arc<Mutex<Vec<String>>> = Arc::default();
    let authed = |h: &HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {MOCK_TOKEN}"));
    let h1 = hits.clone();
    let h2 = hits.clone();
    let old = (Utc::now() - Duration::days(30)).to_rfc3339();
    let app = Router::new()
        .route(
            "/user",
            get(move |h: HeaderMap| async move {
                if !authed(&h) {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"message": "Bad credentials"})));
                }
                (StatusCode::OK, Json(json!({"login": "oleks", "type": "User"})))
            }),
        )
        .route(
            "/issues",
            get(move |h: HeaderMap, Query(q): Query<HashMap<String, String>>| {
                let old = old.clone();
                let hits = h1.clone();
                async move {
                    hits.lock().unwrap().push(format!("issues {}", q.get("filter").cloned().unwrap_or_default()));
                    if !authed(&h) {
                        return (StatusCode::UNAUTHORIZED, Json(json!([])));
                    }
                    if q.get("filter").map(String::as_str) != Some("created") {
                        return (StatusCode::OK, Json(json!([])));
                    }
                    (
                        StatusCode::OK,
                        Json(json!([{
                            "number": 12, "title": "Release checklist for 2.0", "body": "The checklist for the 2.0 release.",
                            "state": "open", "html_url": "https://github.com/kwant/kwant/issues/12",
                            "repository_url": "https://api.github.com/repos/kwant/kwant",
                            "user": {"login": "oleks", "type": "User"}, "assignees": [], "created_at": old, "comments": 1
                        }])),
                    )
                }
            }),
        )
        .route(
            "/repos/kwant/kwant/issues/12/comments",
            get(move || {
                let hits = h2.clone();
                async move {
                    hits.lock().unwrap().push("comments".into());
                    Json(json!([{"user": {"login": "ci[bot]", "type": "Bot"}, "body": "Build passed.", "created_at": (Utc::now() - Duration::days(20)).to_rfc3339()}]))
                }
            }),
        );
    let mut listener = None;
    for port in 18931..18960 {
        if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.expect("a free port in 18931..18960");
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, hits)
}

#[tokio::test]
async fn a_github_source_is_tested_saved_and_scanned() {
    let e = setup().await;
    let (base, hits) = github_mock().await;
    let (s, v) = e.person("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": base}, "secret": "wrong"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a token GitHub refuses is not saved: {v}");
    assert!(e.state.db.zombie_sources().await.unwrap().is_empty());

    let (s, v) = e.person("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": base}, "secret": MOCK_TOKEN}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["source"]["id"].as_str().unwrap().to_string();
    assert!(!v.to_string().contains(MOCK_TOKEN), "the token never comes back");
    let rows = e.state.db.zombie_sources().await.unwrap();
    assert!(!rows[0].config.contains(MOCK_TOKEN), "the token is not in the config");

    let report = kwnt::zombie::hunt::scan(&e.state).await.unwrap();
    let gh = report.sources.iter().find(|s| s.id == id).unwrap();
    assert!(gh.ok, "{report:?}");
    let cases = e.state.db.zombie_cases_of_source(&id).await.unwrap();
    assert_eq!(cases.len(), 1, "{report:?}");
    assert_eq!((cases[0].kind.as_str(), cases[0].reference.as_deref()), ("stale_issue", Some("kwant/kwant#12")));
    assert!(hits.lock().unwrap().iter().any(|h| h == "comments"));

    let (_, v) = e.person("GET", "/api/zombie/sources", None).await;
    let listed = v["sources"].as_array().unwrap().iter().find(|s| s["id"] == id.as_str()).unwrap().clone();
    assert_eq!(listed["last_status"], "ok", "{listed}");

    let (s, v) = e.person("DELETE", &format!("/api/zombie/sources/{id}"), None).await;
    assert_eq!((s, v["cases_removed"].as_u64()), (StatusCode::OK, Some(1)), "{v}");
    let left = e.state.credentials.list(kwnt::zombie::VAULT_OWNER).await.unwrap();
    assert!(left.is_empty(), "the token left the vault with the source");
}

/// F-10 holds for Zombie Hunter too: with approvals off, a mailbox or GitHub login is not saved
/// on this server, where a bot that can run commands could read it. Nothing is tested or kept.
#[tokio::test]
async fn a_source_login_is_refused_on_the_built_in_store_while_approvals_are_off() {
    let e = setup().await;
    assert_eq!(e.state.credentials.store_name(), "local");
    let (base, hits) = github_mock().await;
    let (s, v) = e.person("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": base}, "secret": MOCK_TOKEN}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["source"]["id"].as_str().unwrap().to_string();

    e.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();
    hits.lock().unwrap().clear();
    let (s, v) = e.person("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "Second", "config": {"api_base": base}, "secret": MOCK_TOKEN}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_bitwarden"], true, "{v}");
    assert!(hits.lock().unwrap().is_empty(), "a refused save never reaches the server");
    assert_eq!(e.state.db.zombie_sources().await.unwrap().len(), 1);

    let (s, v) = e.person("PATCH", &format!("/api/zombie/sources/{id}"), Some(json!({"secret": MOCK_TOKEN}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_bitwarden"], true, "{v}");
    assert_eq!(e.state.credentials.list(kwnt::zombie::VAULT_OWNER).await.unwrap().len(), 1, "the saved login is left as it was");
}

/// L-1: editing a source's settings or login with approvals off is refused before any
/// connection is made. A plain TCP listener stands in for the mail and GitHub servers and counts
/// every connection, so a test call that reached it in any way would show.
#[tokio::test]
async fn a_refused_source_edit_never_connects_to_the_server() {
    let e = setup().await;
    let (base, _) = github_mock().await;
    let (s, v) = e.person("POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": base}, "secret": MOCK_TOKEN}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["source"]["id"].as_str().unwrap().to_string();
    let before = e.state.db.zombie_source(&id).await.unwrap().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = connections.clone();
    let accept = tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(sock);
        }
    });
    e.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();

    let trap = format!("http://127.0.0.1:{port}");
    let edits = [
        json!({"config": {"api_base": trap}, "secret": MOCK_TOKEN}),
        json!({"config": {"api_base": trap}}),
        json!({"secret": "a-new-token"}),
    ];
    for body in edits {
        let (s, v) = e.person("PATCH", &format!("/api/zombie/sources/{id}"), Some(body.clone())).await;
        assert_eq!(s, StatusCode::CONFLICT, "{body}: {v}");
        assert_eq!(v["needs_bitwarden"], true, "{v}");
    }
    let imap = json!({"kind": "imap", "label": "Mail", "secret": "dummy-password",
        "config": {"host": "127.0.0.1", "port": port, "security": "plain", "username": "me@example.org"}});
    let (s, v) = e.person("POST", "/api/zombie/sources", Some(imap)).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_bitwarden"], true, "{v}");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 0, "a refused save never reaches the server");
    assert_eq!(e.state.db.zombie_source(&id).await.unwrap().unwrap(), before, "the source is left as it was");
    assert_eq!(e.state.db.zombie_sources().await.unwrap().len(), 1, "no mailbox was added");
    assert_eq!(e.state.credentials.list(kwnt::zombie::VAULT_OWNER).await.unwrap().len(), 1, "the saved login is left as it was");

    // The same edit is tested against the server once approvals are back on
    e.state.db.set_space_approvals(&SpaceId::default_space(), "ask").await.unwrap();
    let (s, v) = e.person("PATCH", &format!("/api/zombie/sources/{id}"), Some(json!({"config": {"api_base": trap}}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(connections.load(std::sync::atomic::Ordering::SeqCst) > 0, "the listener does see a real test");
    accept.abort();
}

/// A case from a bot's chat names the bot, so the Loose ends card can link to the chat.
#[tokio::test]
async fn a_case_from_a_bot_chat_carries_the_bot_id() {
    let e = setup().await;
    let bot = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let thread = e.state.db.create_thread(&bot.id, "Invoice summary", "chat", None, "user").await.unwrap();
    let m = kwnt::db::models::NewMessage {
        channel_id: None,
        direct_kwant_id: Some(bot.id.clone()),
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: "Could you send me the invoice summary?".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = kwnt::db::models::MessageExtra { thread_id: Some(thread.id.clone()), meta: None };
    e.state.db.insert_message_with(&m, &extra).await.unwrap();

    let sources: Vec<Box<dyn MissionSource>> = vec![Box::new(kwnt::zombie::kwant_source::KwantSource::new(e.state.db.clone()))];
    let settings = Settings { kwant_source: true, ..Default::default() };
    scan_with(&e.state.db, &sources, None, &settings, Utc::now() + Duration::days(10)).await.unwrap();
    let cases = e.state.db.zombie_cases_of_source(kwnt::zombie::kwant_source::SOURCE_ID).await.unwrap();
    assert_eq!(cases.len(), 1, "{cases:?}");
    assert_eq!(cases[0].kwant_id.as_deref(), Some(bot.id.as_str()));

    let (s, v) = e.person("GET", &format!("/api/zombie/cases/{}", cases[0].id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["kwant_id"], bot.id.as_str(), "{v}");
    assert_eq!(v["link"], format!("kwant://thread/{}", thread.id), "{v}");
}

/// `kwant_id` came after the first release: a database without it gains the column on start,
/// and the cases already in it read back with no bot.
#[tokio::test]
async fn cases_from_before_kwant_id_survive_the_upgrade() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let key = {
        let db = Database::init(&data).await.unwrap();
        let src = FakeSource::default();
        let now = Utc::now();
        src.set(vec![asked("old", 20, now)]);
        run(&db, &src, None, now).await;
        case(&db, "old").await.dedupe_key
    };
    {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
        sqlx::query("ALTER TABLE zombie_cases DROP COLUMN kwant_id").execute(&pool).await.unwrap();
        pool.close().await;
    }
    let db = Database::init(&data).await.unwrap();
    let mut c = db.zombie_case_by_key(&key).await.unwrap().expect("the old case is still there");
    assert_eq!(c.kwant_id, None);
    c.kwant_id = Some("bot-1".into());
    db.zombie_put_case(&c).await.unwrap();
    assert_eq!(db.zombie_case_by_key(&key).await.unwrap().unwrap().kwant_id.as_deref(), Some("bot-1"));
}

/// The case store is not tied to mail: a later source (a photographed letter, a bank statement)
/// stores its cases in the same table without a schema change.
#[tokio::test]
async fn the_case_store_takes_a_source_that_is_not_mail() {
    let e = setup().await;
    let id = e.seed("letter").await;
    let mut c = e.state.db.zombie_case(&id).await.unwrap().unwrap();
    c.id = "case-camera-1".into();
    c.dedupe_key = "camera:photos:letter-2026-10-20".into();
    c.source = "camera".into();
    c.source_id = "photos".into();
    c.conv_kind = "letter".into();
    c.counterpart = Some("Tax office".into());
    c.deadline = Some("2026-11-15".into());
    c.reply_to_message_id = None;
    e.state.db.zombie_put_case(&c).await.unwrap();
    let back = e.state.db.zombie_case_by_key("camera:photos:letter-2026-10-20").await.unwrap().expect("stored");
    assert_eq!(back, c);
}

// ---- the weekly review ----

#[tokio::test]
async fn a_review_notifies_with_counts_only_and_is_silent_when_empty() {
    let e = setup().await;
    let mut events = e.state.event_sender.subscribe();
    let shown = kwnt::zombie::hunt::review(&e.state, "manual").await.unwrap();
    assert!(shown.is_empty());
    while let Ok(ev) = events.try_recv() {
        assert!(!matches!(ev, kwnt::drivers::events::RuntimeEvent::Notify { .. }), "an empty review stays silent");
    }

    for k in ["a", "b", "c", "d", "e", "f"] {
        e.seed(k).await;
    }
    let shown = kwnt::zombie::hunt::review(&e.state, "manual").await.unwrap();
    assert_eq!(shown.len(), 5, "at most five");
    let mut notified = None;
    while let Ok(ev) = events.try_recv() {
        if let kwnt::drivers::events::RuntimeEvent::Notify { kind, title, body, .. } = ev {
            notified = Some((kind, title, body));
        }
    }
    let (kind, title, body) = notified.expect("a notification");
    assert_eq!(kind, "loose_ends");
    assert_eq!(title, "Loose ends review: 5 things to pick back up");
    assert_eq!(body, "5 waiting on a reply (1 more in the queue).");
    assert!(!body.contains("4471") && !body.contains("Question"), "no subjects or names in a notification");

    let (_, v) = e.person("GET", "/api/zombie/reviews", None).await;
    assert_eq!(v["reviews"][0]["shown"].as_array().map(Vec::len), Some(5), "{v}");
    assert_eq!(v["reviews"][0]["status"], "ok");
}
