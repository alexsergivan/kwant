//! `GET /api/tasks/receipts.csv` (roadmap Next 7): a space's closed tasks as one CSV. Covers
//! escaping and the formula guard end to end, space isolation, the date and state filters, the
//! 10,000-row cap, and a verified task's row (who verified it, the check kinds, the latest verdict).

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        missions::{NewCheck, NewEvidence, NewTask, TaskChange},
        models::Kwant,
        spaces::{NewSpace, SpaceId},
        verifier::NewVerdict,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    raw: SqlitePool,
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
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    // A second connection, only to back-date finished_at and bulk-insert rows
    let opts = SqliteConnectOptions::new().filename(tmp.path().join("kwnt.db")).busy_timeout(std::time::Duration::from_secs(10));
    let raw = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
    Env { app: create_router(state.clone()), state, bot, raw, _tmp: tmp }
}

struct Csv {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
}

impl Csv {
    fn rows(&self) -> Vec<Vec<String>> {
        parse(&self.text)
    }
    /// The data rows as maps by header name.
    fn records(&self) -> Vec<std::collections::HashMap<String, String>> {
        let rows = self.rows();
        let head = rows[0].clone();
        rows[1..].iter().map(|r| head.iter().cloned().zip(r.iter().cloned()).collect()).collect()
    }
    fn ids(&self) -> Vec<String> {
        self.records().into_iter().map(|r| r["task_id"].clone()).collect()
    }
}

async fn get(e: &Env, uri: &str) -> Csv {
    let req = Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Csv { status, headers, text: String::from_utf8(bytes.to_vec()).unwrap() }
}

/// An RFC 4180 reader, to read the export back the way a spreadsheet would.
fn parse(s: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                c => cell.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut cell)),
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    assert!(cell.is_empty() && row.is_empty(), "the export ends with CRLF");
    rows
}

fn at(day: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc()
}

/// A task in `space` moved from backlog to `to`, finished at `finished`.
async fn closed(e: &Env, space: &SpaceId, title: &str, to: &str, finished: DateTime<Utc>) -> String {
    let t = e
        .state
        .db
        .create_task(
            space,
            &NewTask {
                title: title.into(),
                state: "backlog".into(),
                priority: 1,
                owner_kwant_id: Some(e.bot.id.clone()),
                max_attempts: 3,
                source_kind: "person".into(),
                created_by: "person".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    if to == "verified" {
        let c = e
            .state
            .db
            .add_check(&NewCheck {
                task_id: t.id.clone(),
                kind: "person".into(),
                spec: json!({"note": ""}),
                required: true,
                approved: true,
                requested_by: "person".into(),
                approved_by: Some("person".into()),
            })
            .await
            .unwrap();
        e.state.db.finish_check(&c.id, "passed", None, "person", None).await.unwrap();
    }
    let actor = if to == "failed" { "kwant" } else { "person" };
    assert!(e.state.db.move_task(&t.id, "backlog", to, actor, &TaskChange::default()).await.unwrap());
    finish_at(e, &t.id, finished).await;
    t.id
}

async fn finish_at(e: &Env, id: &str, when: DateTime<Utc>) {
    sqlx::query("UPDATE tasks SET finished_at = ? WHERE id = ?").bind(when).bind(id).execute(&e.raw).await.unwrap();
}

#[tokio::test]
async fn a_verified_task_row_names_who_checked_it_how_and_the_latest_verdict() {
    let e = setup().await;
    let space = SpaceId::default_space();
    let id = closed(&e, &space, "Publish the price list", "verified", Utc::now()).await;
    // A kwant check before the person's Verify (both passed), and one that failed
    for (kind, status) in [("command", "passed"), ("http", "failed")] {
        let c = e
            .state
            .db
            .add_check(&NewCheck {
                task_id: id.clone(),
                kind: kind.into(),
                spec: json!({}),
                required: false,
                approved: true,
                requested_by: e.bot.id.clone(),
                approved_by: Some("person".into()),
            })
            .await
            .unwrap();
        e.state.db.finish_check(&c.id, status, None, "kwant", None).await.unwrap();
    }
    for (verdict, summary, couldnt) in [
        ("fail", "old verdict", json!([])),
        ("pass", "Prices match the sheet", json!([{"what": "the PDF export", "why": "no browser"}, "the email"])),
    ] {
        e.state
            .db
            .add_verdict(&NewVerdict {
                task_id: id.clone(),
                attempt: 1,
                round: 1,
                verdict: verdict.into(),
                confidence: "medium".into(),
                risk_tier: "low".into(),
                couldnt_verify: couldnt,
                difference: "none".into(),
                next: "counted".into(),
                summary: summary.into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    for (label, uri, path, sha) in [
        ("=cmd|' /C calc'!A0", Some("https://example.com/prices"), None, Some("0123456789abcdef0123")),
        ("notes, final", None, Some("out/notes.md"), None),
        ("just a note", None, None, None),
    ] {
        e.state
            .db
            .add_evidence(&NewEvidence {
                task_id: id.clone(),
                kind: "note".into(),
                label: label.into(),
                uri: uri.map(Into::into),
                path: path.map(Into::into),
                sha256: sha.map(Into::into),
                excerpt: Some("never exported".into()),
                captured_by: e.bot.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    let csv = get(&e, "/api/tasks/receipts.csv").await;
    assert_eq!(csv.status, StatusCode::OK, "{}", csv.text);
    assert_eq!(csv.headers["content-type"], "text/csv; charset=utf-8");
    let today = Utc::now().date_naive().format("%Y-%m-%d");
    assert_eq!(csv.headers["content-disposition"], format!("attachment; filename=\"kwant-receipts-spc_default-{today}.csv\"").as_str());
    assert!(csv.headers.get("x-kwant-truncated").is_none());
    assert_eq!(
        csv.rows()[0],
        "task_id,title,state,mission,owner_bot,source,created_at,finished_at,verified_by,how_checked,checks_passed,checks_total,verdict,verdict_summary,couldnt_verify,evidence,tokens"
            .split(',')
            .collect::<Vec<_>>()
    );
    let r = &csv.records()[0];
    assert_eq!(r["task_id"], id);
    assert_eq!(r["state"], "verified");
    assert_eq!(r["mission"], "");
    assert_eq!(r["owner_bot"], e.bot.name);
    assert_eq!(r["source"], "person");
    assert_eq!(r["verified_by"], "person");
    assert_eq!(r["how_checked"], "person; command; http");
    assert_eq!((r["checks_passed"].as_str(), r["checks_total"].as_str()), ("2", "3"));
    assert_eq!(r["verdict"], "pass");
    assert_eq!(r["verdict_summary"], "Prices match the sheet");
    assert_eq!(r["couldnt_verify"], "the PDF export (no browser); the email");
    assert_eq!(
        r["evidence"],
        "'=cmd|' /C calc'!A0 (https://example.com/prices, 0123456789ab) | notes, final (out/notes.md) | just a note"
    );
    assert!(!csv.text.contains("never exported"), "no excerpts");
    assert_eq!(r["tokens"], "0");
    assert!(r["finished_at"].ends_with('Z') && r["created_at"].ends_with('Z'));
}

#[tokio::test]
async fn titles_are_escaped_and_formulas_guarded() {
    let e = setup().await;
    let space = SpaceId::default_space();
    let titles = ["Fix \"login\", then deploy", "two\nlines\r\nhere", "=HYPERLINK(\"http://evil\",\"x\")", "+1 call", "-5 items", "@SUM(A1)", "\tindented"];
    for t in titles {
        closed(&e, &space, t, "cancelled", Utc::now()).await;
    }
    let csv = get(&e, "/api/tasks/receipts.csv?state=cancelled").await;
    assert_eq!(csv.status, StatusCode::OK, "{}", csv.text);
    let got: Vec<String> = csv.records().into_iter().map(|r| r["title"].clone()).collect();
    assert_eq!(
        got,
        vec![
            "Fix \"login\", then deploy".to_string(),
            "two\nlines\r\nhere".into(),
            "'=HYPERLINK(\"http://evil\",\"x\")".into(),
            "'+1 call".into(),
            "'-5 items".into(),
            "'@SUM(A1)".into(),
            "'\tindented".into(),
        ]
    );
    assert!(csv.text.contains("\"Fix \"\"login\"\", then deploy\""));
    // Every record has all 17 cells: the newline title stayed in one cell
    assert!(csv.rows().iter().all(|r| r.len() == 17));
}

#[tokio::test]
async fn another_spaces_tasks_never_appear() {
    let e = setup().await;
    let home = SpaceId::default_space();
    let other = e
        .state
        .db
        .create_space(&NewSpace { name: "Clients".into(), icon: "C".into(), color: None, this_machine_enabled: false })
        .await
        .unwrap();
    let other = SpaceId::new(other.id);
    let mine = closed(&e, &home, "mine", "verified", Utc::now()).await;
    let theirs = closed(&e, &other, "theirs", "verified", Utc::now()).await;

    assert_eq!(get(&e, "/api/tasks/receipts.csv").await.ids(), vec![mine.clone()]);
    assert_eq!(get(&e, "/api/tasks/receipts.csv?space=spc_default").await.ids(), vec![mine]);
    let csv = get(&e, &format!("/api/tasks/receipts.csv?space={}", other.as_str())).await;
    assert_eq!(csv.ids(), vec![theirs]);
    assert!(csv.headers["content-disposition"].to_str().unwrap().contains(&format!("kwant-receipts-{}-", other.as_str())));
    assert_eq!(get(&e, "/api/tasks/receipts.csv?space=spc_nope").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_date_and_state_filters_pick_the_rows() {
    let e = setup().await;
    let s = SpaceId::default_space();
    let early = closed(&e, &s, "early", "verified", at("2025-10-01")).await;
    let mid_failed = closed(&e, &s, "mid failed", "failed", at("2025-10-15")).await;
    let mid_cancelled = closed(&e, &s, "mid cancelled", "cancelled", at("2025-10-15")).await;
    let late = closed(&e, &s, "late", "verified", at("2025-11-01")).await;
    // Finished on the last second of `to` counts; midnight after it doesn't
    let edge = closed(&e, &s, "edge", "verified", at("2025-10-31")).await;
    finish_at(&e, &edge, "2025-10-31T23:59:59.999Z".parse().unwrap()).await;
    let after = closed(&e, &s, "after", "verified", at("2025-11-01")).await;
    finish_at(&e, &after, "2025-11-01T00:00:00Z".parse().unwrap()).await;
    // Open tasks never appear
    e.state
        .db
        .create_task(&s, &NewTask { title: "open".into(), state: "backlog".into(), priority: 1, max_attempts: 3, source_kind: "person".into(), created_by: "person".into(), ..Default::default() })
        .await
        .unwrap();

    let all = get(&e, "/api/tasks/receipts.csv?from=2025-09-01&to=2025-12-01").await;
    assert_eq!(all.ids(), vec![early.clone(), mid_failed.clone(), mid_cancelled.clone(), edge.clone(), after.clone(), late.clone()], "oldest first");
    let closed_explicit = get(&e, "/api/tasks/receipts.csv?from=2025-09-01&to=2025-12-01&state=closed").await;
    assert_eq!(closed_explicit.ids(), all.ids());

    assert_eq!(get(&e, "/api/tasks/receipts.csv?from=2025-10-01&to=2025-10-31").await.ids(), vec![early.clone(), mid_failed.clone(), mid_cancelled.clone(), edge.clone()]);
    assert_eq!(get(&e, "/api/tasks/receipts.csv?from=2025-10-02&to=2025-10-31&state=failed").await.ids(), vec![mid_failed]);
    assert_eq!(get(&e, "/api/tasks/receipts.csv?from=2025-10-02&to=2025-10-31&state=cancelled").await.ids(), vec![mid_cancelled]);
    assert_eq!(get(&e, "/api/tasks/receipts.csv?from=2025-09-01&to=2025-12-01&state=verified").await.ids(), vec![early, edge, after, late]);
    // `to` alone: the 30 days before it
    assert_eq!(get(&e, "/api/tasks/receipts.csv?to=2025-10-20").await.ids().len(), 3);
    // The default window (last 30 days) holds none of these old tasks
    let none = get(&e, "/api/tasks/receipts.csv").await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.rows().len(), 1, "header only");

    for (q, says) in [
        ("state=review", "unknown state"),
        ("state=bogus", "unknown state"),
        ("from=2026/10/01", "from must be a date"),
        ("to=yesterday", "to must be a date"),
        ("from=2025-11-02&to=2025-11-01", "from is after to"),
    ] {
        let r = get(&e, &format!("/api/tasks/receipts.csv?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        assert!(r.text.contains(says), "{q}: {}", r.text);
    }
}

#[tokio::test]
async fn more_than_10000_rows_are_cut_and_say_so() {
    let e = setup().await;
    let when = at("2026-10-10");
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10001) \
         INSERT INTO tasks (id, space_id, title, state, budget_tokens, source_kind, created_by, created_at, updated_at, finished_at) \
         SELECT printf('bulk%05d', i), 'spc_default', 'bulk ' || i, 'cancelled', 0, 'person', 'person', ?, ?, ? FROM n",
    )
    .bind(when)
    .bind(when)
    .bind(when)
    .execute(&e.raw)
    .await
    .unwrap();
    let csv = get(&e, "/api/tasks/receipts.csv?from=2026-10-01&to=2026-10-31").await;
    assert_eq!(csv.status, StatusCode::OK);
    assert_eq!(csv.headers["x-kwant-truncated"], "true");
    let ids = csv.ids();
    assert_eq!(ids.len(), 10_000);
    assert_eq!((ids[0].as_str(), ids[9_999].as_str()), ("bulk00001", "bulk10000"));

    let fits = get(&e, "/api/tasks/receipts.csv?from=2026-10-01&to=2026-10-09").await;
    assert!(fits.headers.get("x-kwant-truncated").is_none());
}
