//! A mock GitHub REST API for Dev mode tests (QA plan §2.1), shared by Lin, Vega, Juno and Rigel.
//!
//! ```ignore
//! #[path = "support/mock_github.rs"]
//! mod mock_github;
//! let gh = mock_github::MockGithub::start(&[9481, 9482]).await;   // your own ports; 0 = any
//! gh.json("/repos/demo-org/demo-app", json!({...}));             // 200 + an ETag (304 on a match)
//! gh.fault("/repos/demo-org/demo-app/pulls", Fault::TooMany { retry_after: Some(120) });
//! gh.link_next("/repos/demo-org/demo-app/pulls/7/files", "https://elsewhere.example/page2");
//! ... point kwant at gh.base ...
//! assert!(gh.hits().iter().all(|h| h.authorized));
//! ```
//!
//! - Replies are looked up by the full path with its query first, then by the path alone. A path
//!   with neither gets GitHub's 404 `{"message": "Not Found"}`.
//! - Every 200 carries a strong ETag (a hash of the body); a request whose `If-None-Match` matches
//!   gets 304 with no body, as GitHub does.
//! - With [`MockGithub::require_token`], a request without that bearer token gets 401.
//! - Every request is logged: method, path with query, and whether `Authorization` was sent.
//! - Dev mode reads, and writes only a review the person posts (0.7.1). A request that isn't a GET
//!   is answered 405, logged, and fails the test (dropping the mock panics, unless the test is
//!   already failing), unless the test queued an answer for it with [`MockGithub::accept_write`].
//!   [`MockGithub::assert_read_only`] still panics on any write, accepted or not. Accepted writes
//!   keep their request body: [`MockGithub::writes`].
//! - Secret-looking strings are built at runtime ([`canary_token`], [`secret_like`]), so GitHub push
//!   protection and Gitleaks never see one in the source.

#![allow(dead_code)]

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How a path fails instead of answering.
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// 401 Bad credentials.
    Unauthorized,
    /// A plain 403: no access, and no rate-limit sign.
    Forbidden,
    /// 403 with `x-ratelimit-remaining: 0` and `x-ratelimit-reset` = this epoch second.
    RateLimited { reset_epoch: u64 },
    /// GitHub's secondary limit: 403 with `retry-after` seconds.
    SecondaryLimit { retry_after: u64 },
    /// 429, with or without `retry-after`.
    TooMany { retry_after: Option<u64> },
    /// A 5xx status, e.g. 500, 502 or 503.
    Server(u16),
    /// Waits this long, then answers as usual (the fixture, or 404).
    Slow(Duration),
    /// A compare whose base commit is gone (a force push): 404 "No commit found for SHA".
    MissingBase,
}

/// One request the mock saw.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub method: String,
    /// The path with its query, e.g. `/repos/o/n/commits?sha=main&per_page=50`.
    pub path: String,
    /// Whether an `Authorization` header was sent at all.
    pub authorized: bool,
    /// Whether the reply was a 304.
    pub not_modified: bool,
}

/// A write the mock accepted, with what was sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    pub method: String,
    pub path: String,
    pub authorized: bool,
    /// The request body as sent (kept raw, so a test can search all of it).
    pub body: String,
}

#[derive(Default)]
struct Inner {
    write_replies: Mutex<HashMap<String, std::collections::VecDeque<(u16, Value)>>>,
    writes: Mutex<Vec<Write>>,
    refused_writes: Mutex<Vec<Hit>>,
    replies: Mutex<HashMap<String, Value>>,
    faults: Mutex<HashMap<String, Fault>>,
    links: Mutex<HashMap<String, String>>,
    token: Mutex<Option<String>>,
    hits: Mutex<Vec<Hit>>,
}

pub struct MockGithub {
    /// `http://127.0.0.1:<port>`: the API base to give kwant.
    pub base: String,
    pub port: u16,
    inner: Arc<Inner>,
    server: tokio::task::JoinHandle<()>,
}

impl MockGithub {
    /// Starts on the first free port of `ports` (each team member has their own range), else on
    /// any free port.
    pub async fn start(ports: &[u16]) -> MockGithub {
        let mut listener = None;
        for port in ports.iter().copied().chain([0]) {
            if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                listener = Some(l);
                break;
            }
        }
        let listener = listener.expect("a local port for the mock GitHub");
        let port = listener.local_addr().unwrap().port();
        let inner = Arc::new(Inner::default());
        let app = Router::new().fallback(answer).with_state(inner.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        MockGithub { base: format!("http://127.0.0.1:{port}"), port, inner, server }
    }

    /// 200 with `body` for `path` (with a query to match only that query).
    pub fn json(&self, path: &str, body: Value) -> &Self {
        self.inner.replies.lock().unwrap().insert(path.to_string(), body);
        self
    }

    /// Makes `path` fail. [`MockGithub::heal`] takes it back.
    pub fn fault(&self, path: &str, fault: Fault) -> &Self {
        self.inner.faults.lock().unwrap().insert(path.to_string(), fault);
        self
    }

    pub fn heal(&self, path: &str) -> &Self {
        self.inner.faults.lock().unwrap().remove(path);
        self
    }

    /// Adds `Link: <next>; rel="next"` to `path`'s answer. `next` may be on another host: kwant
    /// must never follow it there.
    pub fn link_next(&self, path: &str, next: &str) -> &Self {
        self.inner.links.lock().unwrap().insert(path.to_string(), next.to_string());
        self
    }

    /// Answers 401 to any request without `Authorization: Bearer <token>`.
    pub fn require_token(&self, token: &str) -> &Self {
        *self.inner.token.lock().unwrap() = Some(token.to_string());
        self
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.inner.hits.lock().unwrap().clone()
    }

    /// The hits whose path (without query) is `path`.
    pub fn hits_of(&self, path: &str) -> Vec<Hit> {
        self.hits().into_iter().filter(|h| h.path.split('?').next() == Some(path)).collect()
    }

    pub fn clear_hits(&self) {
        self.inner.hits.lock().unwrap().clear();
    }

    /// Queues one answer (`status`, `body`) to the next `method` `path` write (the path without
    /// its query). Each queued answer serves one request; a write with none left fails the test.
    pub fn accept_write(&self, method: &str, path: &str, status: u16, body: Value) -> &Self {
        self.inner.write_replies.lock().unwrap().entry(format!("{} {path}", method.to_uppercase())).or_default().push_back((status, body));
        self
    }

    /// The writes answered from [`MockGithub::accept_write`], oldest first.
    pub fn writes(&self) -> Vec<Write> {
        self.inner.writes.lock().unwrap().clone()
    }

    /// Panics if a write reached the mock that no test queued an answer for.
    pub fn assert_no_unexpected_writes(&self) {
        let refused = self.inner.refused_writes.lock().unwrap().clone();
        assert!(refused.is_empty(), "the mock got writes nobody expected: {refused:?}");
    }

    /// Panics if anything but a GET reached the mock.
    pub fn assert_read_only(&self) {
        let writes: Vec<Hit> = self.hits().into_iter().filter(|h| h.method != "GET").collect();
        assert!(writes.is_empty(), "Dev mode only reads GitHub, but the mock got {writes:?}");
    }
}

impl Drop for MockGithub {
    fn drop(&mut self) {
        self.server.abort();
        if !std::thread::panicking() {
            self.assert_no_unexpected_writes();
        }
    }
}

/// A strong ETag for a body, as GitHub sends them.
pub fn etag_of(body: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.to_string().hash(&mut h);
    format!("\"{:016x}\"", h.finish())
}

async fn answer(State(inner): State<Arc<Inner>>, req: Request) -> Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    let full = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| req.uri().path().to_string());
    let path = req.uri().path().to_string();
    let body = axum::body::to_bytes(req.into_body(), 4 << 20).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    fn find<T: Clone>(m: &Mutex<HashMap<String, T>>, full: &str, path: &str) -> Option<T> {
        let m = m.lock().unwrap();
        m.get(full).or_else(|| m.get(path)).cloned()
    }
    let mut hit = Hit { method: method.to_string(), path: full.clone(), authorized: headers.contains_key("authorization"), not_modified: false };

    if method != Method::GET {
        inner.hits.lock().unwrap().push(hit.clone());
        let queued = inner.write_replies.lock().unwrap().get_mut(&format!("{method} {path}")).and_then(|q| q.pop_front());
        let Some((status, reply)) = queued else {
            inner.refused_writes.lock().unwrap().push(hit);
            return (StatusCode::METHOD_NOT_ALLOWED, axum::Json(json!({"message": "the mock GitHub only answers GET"}))).into_response();
        };
        inner.writes.lock().unwrap().push(Write { method: method.to_string(), path: full, authorized: hit.authorized, body });
        return (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), axum::Json(reply)).into_response();
    }
    let token = inner.token.lock().unwrap().clone();
    if let Some(t) = token {
        if bearer(&headers) != Some(t.as_str()) {
            inner.hits.lock().unwrap().push(hit);
            return gh_error(StatusCode::UNAUTHORIZED, "Bad credentials", &[]);
        }
    }
    let fault: Option<Fault> = find(&inner.faults, &full, &path);
    if let Some(Fault::Slow(d)) = &fault {
        tokio::time::sleep(*d).await;
    }
    let reply = match fault {
        Some(Fault::Unauthorized) => Some(gh_error(StatusCode::UNAUTHORIZED, "Bad credentials", &[])),
        Some(Fault::Forbidden) => Some(gh_error(StatusCode::FORBIDDEN, "Resource not accessible by personal access token", &[])),
        Some(Fault::RateLimited { reset_epoch }) => Some(gh_error(
            StatusCode::FORBIDDEN,
            "API rate limit exceeded",
            &[("x-ratelimit-limit", "5000".into()), ("x-ratelimit-remaining", "0".into()), ("x-ratelimit-reset", reset_epoch.to_string())],
        )),
        Some(Fault::SecondaryLimit { retry_after }) => {
            Some(gh_error(StatusCode::FORBIDDEN, "You have exceeded a secondary rate limit", &[("retry-after", retry_after.to_string())]))
        }
        Some(Fault::TooMany { retry_after }) => {
            let h: Vec<(&'static str, String)> = retry_after.map(|s| ("retry-after", s.to_string())).into_iter().collect();
            Some(gh_error(StatusCode::TOO_MANY_REQUESTS, "Too many requests", &h))
        }
        Some(Fault::Server(code)) => Some(gh_error(StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY), "Server Error", &[])),
        Some(Fault::MissingBase) => Some(gh_error(StatusCode::NOT_FOUND, "No commit found for SHA", &[])),
        Some(Fault::Slow(_)) | None => None,
    };
    let reply = reply.unwrap_or_else(|| match find(&inner.replies, &full, &path) {
        None => gh_error(StatusCode::NOT_FOUND, "Not Found", &[]),
        Some(body) => {
            let etag = etag_of(&body);
            if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
                hit.not_modified = true;
                return_304(&etag)
            } else {
                let mut r = (StatusCode::OK, axum::Json(body)).into_response();
                r.headers_mut().insert("etag", HeaderValue::from_str(&etag).unwrap());
                r
            }
        }
    });
    let mut reply = reply;
    if let Some(next) = find(&inner.links, &full, &path) {
        if let Ok(v) = HeaderValue::from_str(&format!("<{next}>; rel=\"next\"")) {
            reply.headers_mut().insert("link", v);
        }
    }
    inner.hits.lock().unwrap().push(hit);
    reply
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get("authorization")?.to_str().ok()?;
    v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("token "))
}

fn return_304(etag: &str) -> Response {
    let mut r = Response::new(Body::empty());
    *r.status_mut() = StatusCode::NOT_MODIFIED;
    r.headers_mut().insert("etag", HeaderValue::from_str(etag).unwrap());
    r
}

fn gh_error(status: StatusCode, message: &str, headers: &[(&'static str, String)]) -> Response {
    let mut r = (status, axum::Json(json!({"message": message, "documentation_url": "https://docs.github.com/rest"}))).into_response();
    for (k, v) in headers {
        r.headers_mut().insert(*k, HeaderValue::from_str(v).unwrap());
    }
    r
}

// ---- Fixtures ---------------------------------------------------------------------------------

/// A GitHub-token-shaped canary (`ghp_` + `CANARY` + 30 chars), unique per call and built at
/// runtime so no scanner finds one in the source.
pub fn canary_token() -> String {
    let tail = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    ["gh", "p_", "CANARY", &tail[..30]].concat()
}

/// `prefix` + `n` random alphanumerics, for any secret shape (`secret_like(&["sk", "_live_"].concat(), 24)`).
pub fn secret_like(prefix: &str, n: usize) -> String {
    let mut s = prefix.to_string();
    while s.len() < prefix.len() + n {
        s.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    s.truncate(prefix.len() + n);
    s
}

pub fn repo(full_name: &str, default_branch: &str) -> Value {
    json!({"full_name": full_name, "default_branch": default_branch, "private": false})
}

pub fn user(login: &str) -> Value {
    json!({"login": login, "type": if login.ends_with("[bot]") { "Bot" } else { "User" }})
}

/// A commit as the list and the single-commit reads return it (`files` only on the latter).
pub fn commit(sha: &str, message: &str, author: (&str, &str), parents: &[&str], date: &str) -> Value {
    let person = json!({"name": author.0, "email": author.1, "date": date});
    json!({
        "sha": sha,
        "html_url": format!("https://github.com/demo-org/demo-app/commit/{sha}"),
        "commit": {"message": message, "author": person, "committer": person},
        "author": null,
        "parents": parents.iter().map(|p| json!({"sha": p})).collect::<Vec<_>>(),
    })
}

pub fn file(name: &str, status: &str, patch: &str) -> Value {
    let adds = patch.lines().filter(|l| l.starts_with('+')).count();
    let dels = patch.lines().filter(|l| l.starts_with('-')).count();
    json!({"filename": name, "status": status, "additions": adds, "deletions": dels, "patch": patch})
}

/// An RFC 3339 time `hours` hours before one moment fixed for the whole test run, as GitHub
/// writes it. Dev mode reviews a branch or PR only while it is fresh (`KWANT_DEV_REVIEW_DAYS`, 7
/// days by default, [`kwnt::server::dev::poller::is_fresh`]), so a hard-coded date turns every
/// review card off a week after the test was written (2026-11-05 fixtures did on 2026-11-12).
/// One base for every call keeps fixtures in the same order and spacing.
pub fn hours_ago(hours: i64) -> String {
    static BASE: std::sync::OnceLock<chrono::DateTime<chrono::Utc>> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(chrono::Utc::now);
    (base - chrono::Duration::hours(hours)).format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A pull request; `merged` gives `(merge_commit_sha, merged_at)`.
pub fn pull(number: u64, title: &str, head: (&str, &str), base: &str, author: &str, merged: Option<(&str, &str)>) -> Value {
    json!({
        "number": number, "title": title, "body": "", "state": if merged.is_some() { "closed" } else { "open" },
        "draft": false, "html_url": format!("https://github.com/demo-org/demo-app/pull/{number}"),
        "user": user(author),
        "head": {"ref": head.0, "sha": head.1}, "base": {"ref": base, "sha": "0000000000000000000000000000000000000000"},
        "merged_at": merged.map(|m| m.1), "merge_commit_sha": merged.map(|m| m.0),
        "merged_by": merged.map(|_| user("demo-person")), "updated_at": hours_ago(1),
    })
}

pub fn check_runs(runs: &[(&str, &str, Option<&str>)]) -> Value {
    let runs: Vec<Value> = runs
        .iter()
        .map(|(name, status, conclusion)| json!({"name": name, "status": status, "conclusion": conclusion, "html_url": "https://github.com/demo-org/demo-app/actions"}))
        .collect();
    json!({"total_count": runs.len(), "check_runs": runs})
}
