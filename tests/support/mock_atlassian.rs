//! A mock Atlassian (Jira and Confluence) for the 0.7.6 jira-tickets tests, shared by Juno, Vega
//! and Rigel. Cloud or Data Center shapes, on 127.0.0.1 and an OS-chosen port. Its Python twin for
//! screenshots is `site-media-kit/rig/mock_atlassian.py`.
//!
//! ```ignore
//! #[path = "support/mock_atlassian.rs"]
//! mod mock_atlassian;
//! let m = mock_atlassian::MockAtlassian::cloud().await;   // or ::dc()
//! m.add_project("TRAIL", "Trail app");
//! let key = m.add_issue("TRAIL", "Add dark mode", &["kwant"], None);
//! // point kwant at m.base (Cloud: email m.email(), token m.token(); DC: PAT m.token())
//! m.assign(&key, Some("kwant-bot")); m.transition(&key, "Done", "done"); m.delete(&key);
//! assert!(m.hits().iter().all(|h| h.auth.is_some() || h.anonymous_ok));
//! ```
//!
//! - **Cloud**: `/rest/api/3/…` at the site with basic `email:token`; a *scoped* token is refused
//!   there (401) and works only through the gateway paths `/ex/jira/<cloudId>/rest/api/3/…` and
//!   `/ex/confluence/<cloudId>/wiki/…` (point `KWANT_ATLASSIAN_GATEWAY_URL` at `base`). Confluence at
//!   `/wiki/…`. `GET /rest/api/3/search` answers 410, as Jira Cloud does since CHANGE-2046.
//! - **DC**: `/rest/api/2/…` with `Bearer <PAT>`; Confluence is "another server" under
//!   `/confluence/…` ([`MockAtlassian::confluence_url`]) with its own PAT. Users have a key
//!   (`JIRAUSER…`, what issues report) and a username (what JQL takes); JQL accepts either.
//! - Anonymous: `/rest/api/2/serverInfo` and (Cloud) `/_edge/tenant_info`.
//! - Search runs a small JQL evaluator: `project in (…)`, `labels =`/`!=`, `assignee =`
//!   (`currentUser()` too), `status =`, `statusCategory =`/`!=`, `key =`, `AND`, `OR`, `NOT`,
//!   parentheses, `ORDER BY` ignored (creation order). Any other field is Jira's 400.
//! - [`MockAtlassian::hide`] keeps an issue out of search (Cloud's search lag) unless its id is in
//!   `reconcileIssues`; [`MockAtlassian::fault`] makes a path fail (429 + Retry-After, 5xx, 401).
//! - Every request is logged ([`Hit`]: method, path, which auth scheme; never the header value).
//!   The only write it accepts is a comment (`POST …/issue/<key>/comment`, kept in
//!   [`MockAtlassian::writes`]); any other write is 405 and lands in [`MockAtlassian::refused`].

#![allow(dead_code)]

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Cloud,
    Dc,
}

/// How a path fails instead of answering.
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// 429, with or without `Retry-After` seconds.
    TooMany { retry_after: Option<u64> },
    /// A 5xx status (503 without Retry-After reads as "unreachable").
    Server(u16),
    /// 401: the token was revoked.
    Unauthorized,
}

/// A Jira user. `handle` is the test's name for them (= the DC username).
#[derive(Clone, Debug, PartialEq)]
pub struct User {
    pub handle: String,
    /// Cloud's accountId.
    pub account_id: String,
    /// DC's user key (what DC issues report).
    pub key: String,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct MockIssue {
    pub id: u64,
    pub key: String,
    pub project: String,
    pub issue_type: String,
    pub summary: String,
    pub description: String,
    pub status: String,
    /// `new`, `indeterminate` or `done`.
    pub category: String,
    pub priority: Option<(String, String)>,
    /// User handles.
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub labels: Vec<String>,
    pub created: String,
    pub updated: String,
    pub deleted: bool,
    /// Out of search results (lag) unless reconciled.
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub struct MockComment {
    pub id: u64,
    pub author: String,
    pub text: String,
    pub created: String,
}

/// One request the mock saw.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub method: String,
    /// With its query.
    pub path: String,
    /// `basic`, `bearer` or `None`.
    pub auth: Option<&'static str>,
    /// Through `/ex/jira/…` or `/ex/confluence/…`.
    pub gateway: bool,
}

/// A comment the mock accepted, with the request body as sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    pub path: String,
    pub body: String,
}

struct Inner {
    flavor: Flavor,
    email: String,
    token: String,
    scoped_token: String,
    confluence_token: String,
    cloud_id: String,
    me: String,
    users: Vec<User>,
    projects: Vec<(String, String)>,
    issues: Vec<MockIssue>,
    comments: HashMap<String, Vec<MockComment>>,
    /// (id, key, name)
    spaces: Vec<(String, String, String)>,
    /// (id, space key, title, storage html)
    pages: Vec<(String, String, String, String)>,
    faults: HashMap<String, Fault>,
    hits: Vec<Hit>,
    writes: Vec<Write>,
    refused: Vec<Hit>,
    reconciled: Vec<Vec<u64>>,
    next_id: u64,
    clock: i64,
    /// Who the test's edits are made as (the changelog's author).
    actor: String,
    /// Per issue key: (created, author handle, fields changed), oldest first.
    changes: HashMap<String, Vec<(String, String, Vec<String>)>>,
}

pub struct MockAtlassian {
    /// `http://127.0.0.1:<port>`: the Jira address (and the gateway) to give kwant.
    pub base: String,
    pub flavor: Flavor,
    inner: Arc<Mutex<Inner>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for MockAtlassian {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Built at runtime so no scanner sees a token-shaped literal.
fn fake(word: &str) -> String {
    format!("mock{}{}", word, "-not-a-real-token")
}

impl MockAtlassian {
    pub async fn cloud() -> Self {
        Self::start(Flavor::Cloud).await
    }

    pub async fn dc() -> Self {
        Self::start(Flavor::Dc).await
    }

    pub async fn start(flavor: Flavor) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a local port for the mock Atlassian");
        let port = listener.local_addr().unwrap().port();
        let me = User { handle: "kwant-bot".into(), account_id: "712020:0a1b2c3d-kwant-bot".into(), key: "JIRAUSER10100".into(), display_name: "kwant bot".into() };
        let inner = Arc::new(Mutex::new(Inner {
            flavor,
            email: "ana@example.com".into(),
            token: fake("classic"),
            scoped_token: fake("scoped"),
            confluence_token: fake("confluence"),
            cloud_id: "a1b2c3d4-0000-4000-8000-00000000c10d".into(),
            me: me.handle.clone(),
            users: vec![me, User { handle: "ana".into(), account_id: "712020:0099-ana".into(), key: "JIRAUSER10099".into(), display_name: "Ana Reporter".into() }],
            projects: Vec::new(),
            issues: Vec::new(),
            comments: HashMap::new(),
            spaces: Vec::new(),
            pages: Vec::new(),
            faults: HashMap::new(),
            hits: Vec::new(),
            writes: Vec::new(),
            refused: Vec::new(),
            reconciled: Vec::new(),
            next_id: 10_000,
            actor: "ana".into(),
            changes: HashMap::new(),
            clock: 0,
        }));
        let app = Router::new().fallback(answer).with_state(inner.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        MockAtlassian { base: format!("http://127.0.0.1:{port}"), flavor, inner, server }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        f(&mut self.inner.lock().unwrap())
    }

    pub fn email(&self) -> String {
        self.with(|i| i.email.clone())
    }
    /// Cloud classic API token, or the DC Jira PAT.
    pub fn token(&self) -> String {
        self.with(|i| i.token.clone())
    }
    /// A Cloud scoped API token: works only through the gateway.
    pub fn scoped_token(&self) -> String {
        self.with(|i| i.scoped_token.clone())
    }
    /// DC Confluence's own PAT.
    pub fn confluence_token(&self) -> String {
        self.with(|i| i.confluence_token.clone())
    }
    pub fn cloud_id(&self) -> String {
        self.with(|i| i.cloud_id.clone())
    }
    /// DC only: Confluence's own address.
    pub fn confluence_url(&self) -> String {
        format!("{}/confluence", self.base)
    }
    /// Revokes every token (all requests 401 from now on).
    pub fn revoke(&self) {
        self.with(|i| {
            i.token = fake("revoked");
            i.scoped_token = fake("revoked2");
        })
    }

    pub fn add_project(&self, key: &str, name: &str) -> &Self {
        self.with(|i| i.projects.push((key.into(), name.into())));
        self
    }

    /// Adds a user; returns it (Cloud accountId, DC key and username).
    pub fn add_user(&self, handle: &str, display_name: &str) -> User {
        self.with(|i| {
            let n = i.users.len() + 100;
            let u = User { handle: handle.into(), account_id: format!("712020:{n:04}-{handle}"), key: format!("JIRAUSER10{n}"), display_name: display_name.into() };
            i.users.push(u.clone());
            u
        })
    }

    pub fn user(&self, handle: &str) -> Option<User> {
        self.with(|i| i.users.iter().find(|u| u.handle == handle).cloned())
    }

    /// The signed-in user (`kwant-bot`).
    pub fn me(&self) -> User {
        self.user("kwant-bot").unwrap()
    }

    /// An open (`To Do`, Medium) issue reported by `ana`, created one minute after the last one.
    /// Returns its key (`<PROJECT>-101`, `-102`, …).
    pub fn add_issue(&self, project: &str, summary: &str, labels: &[&str], assignee: Option<&str>) -> String {
        self.with(|i| {
            i.next_id += 1;
            i.clock += 1;
            let n = i.issues.iter().filter(|x| x.project == project).count() + 101;
            let key = format!("{project}-{n}");
            let t = stamp(i.clock);
            i.issues.push(MockIssue {
                id: i.next_id,
                key: key.clone(),
                project: project.into(),
                issue_type: "Story".into(),
                summary: summary.into(),
                description: format!("What {summary} needs."),
                status: "To Do".into(),
                category: "new".into(),
                priority: Some(("Medium".into(), "3".into())),
                assignee: assignee.map(str::to_string),
                reporter: Some("ana".into()),
                labels: labels.iter().map(|l| l.to_string()).collect(),
                created: t.clone(),
                updated: t,
                deleted: false,
                hidden: false,
            });
            key
        })
    }

    /// Changes an issue (and bumps `updated`).
    pub fn edit(&self, key: &str, f: impl FnOnce(&mut MockIssue)) {
        self.with(|i| {
            i.clock += 1;
            let t = stamp(i.clock);
            let issue = i.issues.iter_mut().find(|x| x.key == key).expect("an issue the test added");
            f(issue);
            issue.updated = t;
        })
    }

    pub fn issue(&self, key: &str) -> Option<MockIssue> {
        self.with(|i| i.issues.iter().find(|x| x.key == key).cloned())
    }

    /// Later edits (assign, labels, transition) are made as `handle` in the changelog.
    pub fn act_as(&self, handle: &str) {
        self.with(|i| i.actor = handle.into());
    }

    fn changed(&self, key: &str, field: &str) {
        self.with(|i| {
            let (t, who) = (stamp(i.clock), i.actor.clone());
            i.changes.entry(key.to_string()).or_default().push((t, who, vec![field.to_string()]));
        })
    }

    pub fn assign(&self, key: &str, handle: Option<&str>) {
        self.edit(key, |x| x.assignee = handle.map(str::to_string));
        self.changed(key, "assignee");
    }
    pub fn set_labels(&self, key: &str, labels: &[&str]) {
        self.edit(key, |x| x.labels = labels.iter().map(|l| l.to_string()).collect());
        self.changed(key, "labels");
    }
    pub fn set_priority(&self, key: &str, name: &str, id: &str) {
        self.edit(key, |x| x.priority = Some((name.into(), id.into())));
    }
    pub fn set_reporter(&self, key: &str, handle: &str) {
        self.edit(key, |x| x.reporter = Some(handle.into()));
    }
    /// `category`: `new`, `indeterminate` or `done`.
    pub fn transition(&self, key: &str, status: &str, category: &str) {
        self.edit(key, |x| {
            x.status = status.into();
            x.category = category.into();
        });
        self.changed(key, "status");
    }
    pub fn delete(&self, key: &str) {
        self.edit(key, |x| x.deleted = true);
    }
    /// Keeps the issue out of search results (Cloud lag) unless its id is reconciled.
    pub fn hide(&self, key: &str, hidden: bool) {
        self.with(|i| {
            if let Some(x) = i.issues.iter_mut().find(|x| x.key == key) {
                x.hidden = hidden;
            }
        })
    }

    /// A comment by `handle` (as if typed in Jira). Returns its id.
    pub fn comment_as(&self, key: &str, handle: &str, text: &str) -> String {
        self.with(|i| {
            i.next_id += 1;
            i.clock += 1;
            let c = MockComment { id: i.next_id, author: handle.into(), text: text.into(), created: stamp(i.clock) };
            i.comments.entry(key.to_string()).or_default().push(c);
            i.next_id.to_string()
        })
    }

    pub fn comments(&self, key: &str) -> Vec<MockComment> {
        self.with(|i| i.comments.get(key).cloned().unwrap_or_default())
    }

    pub fn add_space(&self, key: &str, name: &str) -> &Self {
        self.with(|i| {
            let id = (i.spaces.len() as u64 + 98_000).to_string();
            i.spaces.push((id, key.into(), name.into()));
        });
        self
    }

    /// A page in `space` with `html` as its storage body. Returns its id.
    pub fn add_page(&self, space: &str, title: &str, html: &str) -> String {
        self.with(|i| {
            i.next_id += 1;
            let id = (i.next_id + 300_000).to_string();
            i.pages.push((id.clone(), space.into(), title.into(), html.into()));
            id
        })
    }

    /// Makes requests whose API path ends with `suffix` fail (`*` = every request).
    pub fn fault(&self, suffix: &str, fault: Fault) -> &Self {
        self.with(|i| i.faults.insert(suffix.to_string(), fault));
        self
    }

    pub fn heal(&self, suffix: &str) -> &Self {
        self.with(|i| i.faults.remove(suffix));
        self
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.with(|i| i.hits.clone())
    }
    pub fn clear_hits(&self) {
        self.with(|i| i.hits.clear())
    }
    pub fn writes(&self) -> Vec<Write> {
        self.with(|i| i.writes.clone())
    }
    pub fn refused(&self) -> Vec<Hit> {
        self.with(|i| i.refused.clone())
    }
    /// The `reconcileIssues` of each Cloud search, in order.
    pub fn reconciled(&self) -> Vec<Vec<u64>> {
        self.with(|i| i.reconciled.clone())
    }
    /// Searches run so far.
    pub fn searches(&self) -> usize {
        self.hits().iter().filter(|h| h.path.contains("/search")).count()
    }
}

/// `2026-11-01T09:<n>:00.000+0000`-style times, `n` minutes after 09:00 on 1 November 2026.
fn stamp(n: i64) -> String {
    let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(9, 0, 0).unwrap() + chrono::Duration::minutes(n);
    t.format("%Y-%m-%dT%H:%M:%S%.3f+0000").to_string()
}

fn reply(status: u16, body: Value) -> Response {
    Response::builder().status(status).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
}

fn not_found() -> Response {
    reply(404, json!({"errorMessages": ["Issue does not exist or you do not have permission to see it."], "errors": {}}))
}

fn adf(text: &str) -> Value {
    json!({"type": "doc", "version": 1, "content": text.split("\n\n").map(|p| json!({"type": "paragraph", "content": [{"type": "text", "text": p}]})).collect::<Vec<_>>()})
}

impl Inner {
    fn user(&self, handle: &str) -> Option<&User> {
        self.users.iter().find(|u| u.handle == handle)
    }

    fn person(&self, handle: Option<&str>) -> Value {
        let Some(u) = handle.and_then(|h| self.user(h)) else { return Value::Null };
        match self.flavor {
            Flavor::Cloud => json!({"accountId": u.account_id, "displayName": u.display_name, "active": true, "accountType": "atlassian"}),
            Flavor::Dc => json!({"key": u.key, "name": u.handle, "displayName": u.display_name, "active": true}),
        }
    }

    /// Whether `value` (from JQL) names `handle`.
    fn names(&self, handle: Option<&str>, value: &str) -> bool {
        let Some(u) = handle.and_then(|h| self.user(h)) else { return false };
        if value.eq_ignore_ascii_case("currentUser()") {
            return u.handle == self.me;
        }
        match self.flavor {
            Flavor::Cloud => u.account_id == value,
            Flavor::Dc => u.handle == value || u.key == value,
        }
    }

    fn issue_json(&self, x: &MockIssue, base: &str) -> Value {
        let description = match self.flavor {
            Flavor::Cloud => adf(&x.description),
            Flavor::Dc => Value::String(x.description.clone()),
        };
        json!({
            "id": x.id.to_string(),
            "key": x.key,
            "self": format!("{base}/rest/api/2/issue/{}", x.id),
            "fields": {
                "summary": x.summary,
                "description": description,
                "status": {"name": x.status, "statusCategory": {"key": x.category}},
                "priority": x.priority.as_ref().map(|(n, id)| json!({"name": n, "id": id})),
                "assignee": self.person(x.assignee.as_deref()),
                "reporter": self.person(x.reporter.as_deref()),
                "labels": x.labels,
                "created": x.created,
                "updated": x.updated,
                "project": {"key": x.project, "name": self.projects.iter().find(|p| p.0 == x.project).map(|p| p.1.clone()).unwrap_or_default()},
                "issuetype": {"name": x.issue_type},
                "parent": null,
                "issuelinks": [],
                "attachment": [],
            }
        })
    }

    fn comment_json(&self, c: &MockComment) -> Value {
        let body = match self.flavor {
            Flavor::Cloud => adf(&c.text),
            Flavor::Dc => Value::String(c.text.clone()),
        };
        json!({"id": c.id.to_string(), "author": self.person(Some(&c.author)), "body": body, "created": c.created, "updated": c.created})
    }
}

/// Which product and auth realm a path belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Realm {
    Jira,
    Confluence,
}

fn basic_of(headers: &HeaderMap) -> Option<(String, String)> {
    use base64::Engine;
    let h = headers.get("authorization")?.to_str().ok()?;
    let raw = base64::engine::general_purpose::STANDARD.decode(h.strip_prefix("Basic ")?).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (e, t) = s.split_once(':')?;
    Some((e.to_string(), t.to_string()))
}

fn bearer_of(headers: &HeaderMap) -> Option<String> {
    headers.get("authorization")?.to_str().ok()?.strip_prefix("Bearer ").map(str::to_string)
}

async fn answer(State(inner): State<Arc<Mutex<Inner>>>, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();
    let host = headers.get("host").and_then(|h| h.to_str().ok()).unwrap_or("127.0.0.1").to_string();
    let base = format!("http://{host}");
    let body = to_bytes(req.into_body(), 1 << 20).await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let path = uri.path().to_string();
    let query: HashMap<String, String> = uri.query().map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default();
    let full = uri.path_and_query().map(|p| p.to_string()).unwrap_or_else(|| path.clone());

    let mut i = inner.lock().unwrap();
    let auth = headers.get("authorization").and_then(|h| h.to_str().ok()).map(|h| if h.starts_with("Basic ") { "basic" } else if h.starts_with("Bearer ") { "bearer" } else { "other" });
    let gateway = path.starts_with("/ex/");
    let hit = Hit { method: method.to_string(), path: full.clone(), auth, gateway };
    i.hits.push(hit.clone());

    // Anonymous endpoints. `fault("/serverInfo", …)` reaches the detection too.
    if path == "/rest/api/2/serverInfo" {
        if let Some(f) = i.faults.get("/serverInfo").cloned() {
            return fault_reply(f);
        }
        return match i.flavor {
            Flavor::Cloud => reply(200, json!({"baseUrl": base, "version": "1001.0.0-SNAPSHOT", "deploymentType": "Cloud"})),
            Flavor::Dc => reply(200, json!({"baseUrl": base, "version": "9.12.4", "deploymentType": "Server"})),
        };
    }
    if path == "/_edge/tenant_info" {
        return match i.flavor {
            Flavor::Cloud => reply(200, json!({"cloudId": i.cloud_id})),
            Flavor::Dc => reply(404, json!({})),
        };
    }

    // Which realm, and the API path inside it.
    let (realm, rest) = if let Some(r) = path.strip_prefix("/ex/jira/") {
        let (id, r) = r.split_once('/').unwrap_or((r, ""));
        if i.flavor != Flavor::Cloud || id != i.cloud_id {
            return reply(404, json!({"message": "no such site"}));
        }
        (Realm::Jira, format!("/{r}"))
    } else if let Some(r) = path.strip_prefix("/ex/confluence/") {
        let (id, r) = r.split_once('/').unwrap_or((r, ""));
        if i.flavor != Flavor::Cloud || id != i.cloud_id {
            return reply(404, json!({"message": "no such site"}));
        }
        (Realm::Confluence, format!("/{r}"))
    } else if path.starts_with("/wiki/") && i.flavor == Flavor::Cloud {
        (Realm::Confluence, path.clone())
    } else if let Some(r) = path.strip_prefix("/confluence") {
        if i.flavor != Flavor::Dc {
            return reply(404, json!({}));
        }
        (Realm::Confluence, r.to_string())
    } else {
        (Realm::Jira, path.clone())
    };

    // Paths that aren't Jira or Confluence REST are a plain web server's 404 (no auth asked).
    if !(rest.starts_with("/rest/") || (matches!(realm, Realm::Confluence) && rest.starts_with("/wiki/"))) {
        return reply(404, json!({}));
    }

    // Auth.
    let ok = match (i.flavor, realm) {
        (Flavor::Cloud, _) => basic_of(&headers).is_some_and(|(e, t)| e == i.email && (t == i.token || (gateway && t == i.scoped_token))),
        (Flavor::Dc, Realm::Jira) => bearer_of(&headers).is_some_and(|t| t == i.token),
        (Flavor::Dc, Realm::Confluence) => bearer_of(&headers).is_some_and(|t| t == i.confluence_token),
    };
    if !ok {
        return reply(401, json!({"errorMessages": ["You are not authenticated. Authentication required to perform this operation."]}));
    }

    if let Some(f) = i.faults.iter().find(|(k, _)| k.as_str() == "*" || rest.ends_with(k.as_str())).map(|(_, f)| f.clone()) {
        return fault_reply(f);
    }

    match realm {
        Realm::Jira => jira(&mut i, &method, &rest, &query, &body, &base, hit),
        Realm::Confluence => confluence(&mut i, &method, &rest, &query, &base),
    }
}

fn fault_reply(f: Fault) -> Response {
    match f {
        Fault::TooMany { retry_after } => {
            let mut r = Response::builder().status(429).header("content-type", "application/json");
            if let Some(s) = retry_after {
                r = r.header("retry-after", s.to_string());
            }
            r.body(Body::from(json!({"errorMessages": ["Rate limit exceeded."]}).to_string())).unwrap()
        }
        Fault::Server(code) => reply(code, json!({"errorMessages": ["Service unavailable"]})),
        Fault::Unauthorized => reply(401, json!({"errorMessages": ["Unauthorized"]})),
    }
}

fn jira(i: &mut Inner, method: &Method, rest: &str, query: &HashMap<String, String>, body: &Value, base: &str, hit: Hit) -> Response {
    let want = match i.flavor {
        Flavor::Cloud => "/rest/api/3",
        Flavor::Dc => "/rest/api/2",
    };
    let Some(api) = rest.strip_prefix(want) else {
        return reply(404, json!({"errorMessages": ["No such API version here."]}));
    };
    let parts: Vec<&str> = api.trim_start_matches('/').split('/').collect();
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["myself"]) => {
            let me = i.me.clone();
            let mut v = i.person(Some(&me));
            v["emailAddress"] = json!(i.email);
            reply(200, v)
        }
        ("GET", ["project", "search"]) if i.flavor == Flavor::Cloud => {
            let values: Vec<Value> = i.projects.iter().map(|(k, n)| json!({"key": k, "name": n})).collect();
            reply(200, json!({"values": values, "isLast": true, "startAt": 0, "total": i.projects.len()}))
        }
        ("GET", ["project"]) if i.flavor == Flavor::Dc => reply(200, Value::Array(i.projects.iter().map(|(k, n)| json!({"key": k, "name": n})).collect())),
        ("GET", ["field"]) => match i.flavor {
            Flavor::Dc => reply(200, json!([{"id": "customfield_10100", "name": "Epic Link"}, {"id": "summary", "name": "Summary"}])),
            Flavor::Cloud => reply(200, json!([{"id": "summary", "name": "Summary"}])),
        },
        ("GET", ["user", "assignable", "multiProjectSearch"]) => {
            let q = query.get("query").or(query.get("username")).map(|s| s.to_lowercase()).unwrap_or_default();
            let handles: Vec<String> = i.users.iter().filter(|u| q.is_empty() || u.handle.to_lowercase().contains(&q) || u.display_name.to_lowercase().contains(&q)).map(|u| u.handle.clone()).collect();
            reply(200, Value::Array(handles.iter().map(|h| i.person(Some(h))).collect()))
        }
        ("POST", ["search", "jql"]) if i.flavor == Flavor::Cloud => {
            let reconcile: Vec<u64> = body["reconcileIssues"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
            if reconcile.len() > 50 {
                return reply(400, json!({"errorMessages": ["reconcileIssues takes at most 50 ids."]}));
            }
            i.reconciled.push(reconcile.clone());
            let hits = match search(i, body["jql"].as_str().unwrap_or_default(), &reconcile) {
                Ok(h) => h,
                Err(m) => return reply(400, json!({"errorMessages": [m], "errors": {}})),
            };
            let per = body["maxResults"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
            let start = body["nextPageToken"].as_str().and_then(|t| t.strip_prefix("page-")).and_then(|t| t.parse::<usize>().ok()).unwrap_or(0);
            let end = (start + per).min(hits.len());
            let issues: Vec<Value> = hits[start.min(end)..end].iter().map(|x| i.issue_json(x, base)).collect();
            let mut v = json!({"issues": issues, "isLast": end >= hits.len()});
            if end < hits.len() {
                v["nextPageToken"] = json!(format!("page-{end}"));
            }
            reply(200, v)
        }
        ("POST", ["search", "approximate-count"]) if i.flavor == Flavor::Cloud => match search(i, body["jql"].as_str().unwrap_or_default(), &[]) {
            Ok(h) => reply(200, json!({"count": h.len()})),
            Err(m) => reply(400, json!({"errorMessages": [m], "errors": {}})),
        },
        ("GET" | "POST", ["search"]) if i.flavor == Flavor::Cloud => reply(410, json!({"errorMessages": ["The requested API has been removed. Please migrate to the /rest/api/3/search/jql API."]})),
        ("POST", ["search"]) => {
            let hits = match search(i, body["jql"].as_str().unwrap_or_default(), &[]) {
                Ok(h) => h,
                Err(m) => return reply(400, json!({"errorMessages": [m], "errors": {}})),
            };
            let start = body["startAt"].as_u64().unwrap_or(0) as usize;
            let per = body["maxResults"].as_u64().unwrap_or(50).min(100) as usize;
            let end = (start + per).min(hits.len());
            let issues: Vec<Value> = hits[start.min(end)..end].iter().map(|x| i.issue_json(x, base)).collect();
            reply(200, json!({"startAt": start, "maxResults": per, "total": hits.len(), "issues": issues}))
        }
        ("GET", ["issue", key]) => match i.issues.iter().find(|x| x.key == *key && !x.deleted) {
            Some(x) => {
                let mut v = i.issue_json(x, base);
                if query.get("expand").is_some_and(|e| e.split(',').any(|p| p.trim() == "changelog")) {
                    // Newest first, as Jira Cloud serves them; kwant must not rely on the order.
                    let histories: Vec<Value> = i
                        .changes
                        .get(*key)
                        .into_iter()
                        .flatten()
                        .rev()
                        .map(|(t, who, fields)| json!({"author": i.person(Some(who)), "created": t, "items": fields.iter().map(|f| json!({"field": f})).collect::<Vec<_>>()}))
                        .collect();
                    v["changelog"] = json!({"startAt": 0, "maxResults": histories.len(), "total": histories.len(), "histories": histories});
                }
                reply(200, v)
            }
            None => not_found(),
        },
        ("GET", ["issue", key, "remotelink"]) => match i.issues.iter().any(|x| x.key == *key && !x.deleted) {
            true => reply(200, json!([])),
            false => not_found(),
        },
        ("GET", ["issue", key, "comment"]) => {
            if !i.issues.iter().any(|x| x.key == *key && !x.deleted) {
                return not_found();
            }
            let list: Vec<Value> = i.comments.get(*key).cloned().unwrap_or_default().iter().map(|c| i.comment_json(c)).collect();
            reply(200, json!({"startAt": 0, "maxResults": 100, "total": list.len(), "comments": list}))
        }
        ("POST", ["issue", key, "comment"]) => {
            if !i.issues.iter().any(|x| x.key == *key && !x.deleted) {
                return not_found();
            }
            let text = match (i.flavor, &body["body"]) {
                (Flavor::Cloud, b) if b["type"] == "doc" => adf_text(b),
                (Flavor::Dc, Value::String(s)) => s.clone(),
                _ => return reply(400, json!({"errorMessages": ["Comment body is not valid."], "errors": {"comment": "Comment body can not be empty!"}})),
            };
            i.writes.push(Write { path: hit.path.clone(), body: body.to_string() });
            i.next_id += 1;
            i.clock += 1;
            let c = MockComment { id: i.next_id, author: i.me.clone(), text, created: stamp(i.clock) };
            i.comments.entry(key.to_string()).or_default().push(c.clone());
            reply(201, i.comment_json(&c))
        }
        ("GET", _) => reply(404, json!({"errorMessages": ["Not found."]})),
        _ => {
            i.refused.push(hit);
            reply(405, json!({"errorMessages": ["The mock Atlassian accepts no write but a comment."]}))
        }
    }
}

fn adf_text(v: &Value) -> String {
    v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["content"].as_array().into_iter().flatten().filter_map(|t| t["text"].as_str().or(t["attrs"]["text"].as_str())).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn confluence(i: &mut Inner, method: &Method, rest: &str, query: &HashMap<String, String>, base: &str) -> Response {
    if method != Method::GET {
        return reply(405, json!({"message": "read only"}));
    }
    let space_json = |s: &(String, String, String)| json!({"id": s.0, "key": s.1, "name": s.2, "type": "global"});
    let words = |cql: &str| -> (Vec<String>, String) {
        // `space in ("A","B") AND type = page AND (text ~ "w" OR title ~ "w")` — enough for kwant's own CQL.
        let spaces = cql.split("space in (").nth(1).and_then(|r| r.split(')').next()).map(|r| r.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect()).unwrap_or_default();
        let w = cql.split('~').nth(1).map(|r| r.trim().trim_start_matches('"').split('"').next().unwrap_or("").to_lowercase()).unwrap_or_default();
        (spaces, w)
    };
    let page_hit = |p: &(String, String, String, String)| p.clone();
    match (i.flavor, rest) {
        (Flavor::Cloud, "/wiki/api/v2/spaces") => {
            let keys: Option<Vec<String>> = query.get("keys").map(|k| k.split(',').map(str::to_string).collect());
            let list: Vec<Value> = i.spaces.iter().filter(|s| keys.as_ref().is_none_or(|k| k.contains(&s.1))).map(space_json).collect();
            reply(200, json!({"results": list}))
        }
        (Flavor::Dc, "/rest/api/space") => reply(200, json!({"results": i.spaces.iter().map(space_json).collect::<Vec<_>>()})),
        (Flavor::Cloud, "/wiki/rest/api/search") | (Flavor::Dc, "/rest/api/content/search") => {
            let (spaces, w) = words(query.get("cql").map(String::as_str).unwrap_or_default());
            let found: Vec<_> = i.pages.iter().filter(|p| spaces.contains(&p.1) && (w.is_empty() || p.2.to_lowercase().contains(&w) || p.3.to_lowercase().contains(&w))).map(page_hit).collect();
            let results: Vec<Value> = found
                .iter()
                .map(|(id, space, title, html)| {
                    let content = json!({"id": id, "type": "page", "title": title, "space": {"key": space}, "version": {"number": 3, "when": "2026-11-01T09:00:00.000Z"}});
                    match i.flavor {
                        Flavor::Cloud => json!({"content": content, "excerpt": html.chars().take(120).collect::<String>(), "lastModified": "2026-11-01T09:00:00.000Z"}),
                        Flavor::Dc => content,
                    }
                })
                .collect();
            reply(200, json!({"results": results}))
        }
        (Flavor::Cloud, r) if r.starts_with("/wiki/api/v2/pages/") => {
            let id = r.trim_start_matches("/wiki/api/v2/pages/");
            match i.pages.iter().find(|p| p.0 == id) {
                Some((id, space, title, html)) => {
                    let space_id = i.spaces.iter().find(|s| &s.1 == space).map(|s| s.0.clone()).unwrap_or_default();
                    reply(200, json!({"id": id, "title": title, "spaceId": space_id, "version": {"number": 3}, "body": {"storage": {"value": html, "representation": "storage"}}}))
                }
                None => reply(404, json!({"message": "not found"})),
            }
        }
        (Flavor::Dc, r) if r.starts_with("/rest/api/content/") => {
            let id = r.trim_start_matches("/rest/api/content/");
            match i.pages.iter().find(|p| p.0 == id) {
                Some((id, space, title, html)) => reply(200, json!({"id": id, "type": "page", "title": title, "space": {"key": space}, "version": {"number": 3}, "body": {"storage": {"value": html}}, "_links": {"base": base}})),
                None => reply(404, json!({"message": "not found"})),
            }
        }
        _ => reply(404, json!({"message": "not found"})),
    }
}

// ---- A small JQL evaluator: enough for kwant's own JQL and the tests' Advanced JQL ----

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Open,
    Close,
    Comma,
    Eq,
    Ne,
}

fn tokens(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < c.len() {
        match c[k] {
            ' ' | '\t' | '\n' => k += 1,
            '(' => {
                out.push(Tok::Open);
                k += 1
            }
            ')' => {
                out.push(Tok::Close);
                k += 1
            }
            ',' => {
                out.push(Tok::Comma);
                k += 1
            }
            '=' => {
                out.push(Tok::Eq);
                k += 1
            }
            '!' if c.get(k + 1) == Some(&'=') => {
                out.push(Tok::Ne);
                k += 2
            }
            q @ ('"' | '\'') => {
                let mut v = String::new();
                k += 1;
                while k < c.len() && c[k] != q {
                    if c[k] == '\\' && k + 1 < c.len() {
                        k += 1;
                    }
                    v.push(c[k]);
                    k += 1;
                }
                if k >= c.len() {
                    return Err("The quoted string has no end.".into());
                }
                k += 1;
                out.push(Tok::Str(v));
            }
            _ => {
                let mut v = String::new();
                while k < c.len() && (c[k].is_alphanumeric() || matches!(c[k], '_' | '-' | '.' | ':')) {
                    v.push(c[k]);
                    k += 1;
                }
                if v.is_empty() {
                    return Err(format!("Error in the JQL Query: The character '{}' is a reserved JQL character.", c[k]));
                }
                if v.eq_ignore_ascii_case("currentUser") && c.get(k) == Some(&'(') && c.get(k + 1) == Some(&')') {
                    v.push_str("()");
                    k += 2;
                }
                out.push(Tok::Word(v));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Q {
    And(Box<Q>, Box<Q>),
    Or(Box<Q>, Box<Q>),
    Not(Box<Q>),
    /// field, negated, values
    Cond(String, bool, Vec<String>),
}

struct P {
    t: Vec<Tok>,
    k: usize,
}

impl P {
    fn peek_word(&self, w: &str) -> bool {
        matches!(self.t.get(self.k), Some(Tok::Word(x)) if x.eq_ignore_ascii_case(w))
    }
    fn expr(&mut self) -> Result<Q, String> {
        let mut q = self.and()?;
        while self.peek_word("OR") {
            self.k += 1;
            q = Q::Or(Box::new(q), Box::new(self.and()?));
        }
        Ok(q)
    }
    fn and(&mut self) -> Result<Q, String> {
        let mut q = self.unary()?;
        while self.peek_word("AND") {
            self.k += 1;
            q = Q::And(Box::new(q), Box::new(self.unary()?));
        }
        Ok(q)
    }
    fn unary(&mut self) -> Result<Q, String> {
        if self.peek_word("NOT") {
            self.k += 1;
            return Ok(Q::Not(Box::new(self.unary()?)));
        }
        if self.t.get(self.k) == Some(&Tok::Open) {
            self.k += 1;
            let q = self.expr()?;
            if self.t.get(self.k) != Some(&Tok::Close) {
                return Err("Error in the JQL Query: Expecting ')'.".into());
            }
            self.k += 1;
            return Ok(q);
        }
        let field = match self.t.get(self.k) {
            Some(Tok::Word(f)) => f.clone(),
            _ => return Err("Error in the JQL Query: Expecting a field name.".into()),
        };
        self.k += 1;
        let known = ["project", "labels", "assignee", "status", "statuscategory", "key", "issuekey", "issuetype", "type", "reporter"];
        if !known.contains(&field.to_ascii_lowercase().as_str()) {
            return Err(format!("Field '{field}' does not exist or you do not have permission to view it."));
        }
        let neg = match self.t.get(self.k) {
            Some(Tok::Eq) => false,
            Some(Tok::Ne) => true,
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("in") => false,
            Some(Tok::Word(w)) if w.eq_ignore_ascii_case("not") => {
                self.k += 1;
                true
            }
            _ => return Err(format!("Error in the JQL Query: Expecting an operator after '{field}'.")),
        };
        let is_in = self.peek_word("in");
        self.k += 1;
        let mut values = Vec::new();
        if is_in {
            if self.t.get(self.k) != Some(&Tok::Open) {
                return Err("Error in the JQL Query: Expecting '(' after IN.".into());
            }
            self.k += 1;
            loop {
                match self.t.get(self.k) {
                    Some(Tok::Word(v)) | Some(Tok::Str(v)) => values.push(v.clone()),
                    _ => return Err("Error in the JQL Query: Expecting a value.".into()),
                }
                self.k += 1;
                match self.t.get(self.k) {
                    Some(Tok::Comma) => self.k += 1,
                    Some(Tok::Close) => {
                        self.k += 1;
                        break;
                    }
                    _ => return Err("Error in the JQL Query: Expecting ',' or ')'.".into()),
                }
            }
        } else {
            match self.t.get(self.k) {
                Some(Tok::Word(v)) | Some(Tok::Str(v)) => values.push(v.clone()),
                _ => return Err("Error in the JQL Query: Expecting a value.".into()),
            }
            self.k += 1;
        }
        Ok(Q::Cond(field.to_ascii_lowercase(), neg, values))
    }
}

fn parse_jql(jql: &str) -> Result<Q, String> {
    let lower = jql.to_ascii_lowercase();
    let body = match lower.rfind("order by") {
        Some(at) => &jql[..at],
        None => jql,
    };
    let mut p = P { t: tokens(body)?, k: 0 };
    let q = p.expr()?;
    if p.k != p.t.len() {
        return Err("Error in the JQL Query: Expecting either 'OR' or 'AND' but got more.".into());
    }
    Ok(q)
}

fn category_key(v: &str) -> String {
    match v.to_ascii_lowercase().as_str() {
        "done" => "done".into(),
        "to do" | "new" => "new".into(),
        "in progress" | "indeterminate" => "indeterminate".into(),
        other => other.into(),
    }
}

fn eval(i: &Inner, q: &Q, x: &MockIssue) -> bool {
    match q {
        Q::And(a, b) => eval(i, a, x) && eval(i, b, x),
        Q::Or(a, b) => eval(i, a, x) || eval(i, b, x),
        Q::Not(a) => !eval(i, a, x),
        Q::Cond(field, neg, values) => {
            let any = |f: &dyn Fn(&str) -> bool| values.iter().any(|v| f(v));
            let hit = match field.as_str() {
                "project" => any(&|v| v.eq_ignore_ascii_case(&x.project)),
                "labels" => any(&|v| x.labels.iter().any(|l| l == v)),
                "assignee" => any(&|v| i.names(x.assignee.as_deref(), v)),
                "reporter" => any(&|v| i.names(x.reporter.as_deref(), v)),
                "status" => any(&|v| v.eq_ignore_ascii_case(&x.status)),
                "statuscategory" => any(&|v| category_key(v) == x.category),
                "key" | "issuekey" => any(&|v| v.eq_ignore_ascii_case(&x.key)),
                "issuetype" | "type" => any(&|v| v.eq_ignore_ascii_case(&x.issue_type)),
                _ => false,
            };
            hit != *neg
        }
    }
}

fn search(i: &Inner, jql: &str, reconcile: &[u64]) -> Result<Vec<MockIssue>, String> {
    let q = parse_jql(jql)?;
    Ok(i.issues.iter().filter(|x| !x.deleted && (!x.hidden || reconcile.contains(&x.id)) && eval(i, &q, x)).cloned().collect())
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn jql_parses_kwants_own_shapes() {
        for j in [
            "project in (\"TRAIL\") AND labels = \"kwant\" AND statusCategory != Done ORDER BY created ASC",
            "project in (\"TRAIL\", \"HR\") AND (assignee = \"x\" OR labels = \"kwant\") AND statusCategory != Done ORDER BY created ASC",
            "project in (\"TRAIL\") AND (labels = kwant OR project = HR) ORDER BY created ASC",
            "assignee = currentUser() AND NOT status = Done",
        ] {
            parse_jql(j).unwrap_or_else(|e| panic!("{j}: {e}"));
        }
        assert!(parse_jql("project in (\"TRAIL\") AND priorityx = High").unwrap_err().contains("does not exist"));
    }
}
