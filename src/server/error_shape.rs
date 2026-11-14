//! One shape for every refusal under `/api/` (roadmap 14d): `{error, code[, detail]}`.
//!
//! Handlers keep their own messages and codes; this layer only fills the gaps, on 4xx/5xx
//! responses to `/api/` paths:
//! - **axum's own extractor rejections** (`Query`, `Json`, `Path`, body limits), which axum
//!   answers in plain text ("Failed to deserialize query string"), become a plain sentence with a
//!   code (`bad_request`, `bad_json`, `unsupported_media`, `too_large`, `unprocessable`); the
//!   parser's text is kept in `detail`, cut to [`DETAIL_CHARS`] characters.
//! - **another plain-text or empty body** (a handler's own `(StatusCode, String)`) keeps its text
//!   as `error` (the status's sentence when empty) and gets the status's code.
//! - **a JSON body with a string `error` and no `code`** (the per-module `error()` helpers) gets
//!   the status's code ([`status_code`]); every other field stays as it is.
//!
//! Everything else passes through untouched: success responses, bodies that already have a
//! `code`, JSON that isn't `{error: "…"}` (JSON-RPC answers included), any other content type
//! (downloads, HTML, event streams), and bodies that may be larger than [`MAX_BODY`] or have no
//! known length (a stream is never buffered).

use axum::{
    body::{to_bytes, Body, HttpBody},
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use serde_json::{json, Map, Value};

/// Error bodies at most this large are read; a larger or unknown-length one passes untouched.
pub const MAX_BODY: u64 = 64 * 1024;
/// The parser's own text is kept in `detail` up to this many characters.
pub const DETAIL_CHARS: usize = 300;

/// The texts axum answers its extractor rejections with (axum 0.7 / axum-core 0.4), by prefix.
const REJECTION_PREFIXES: [&str; 16] = [
    "Failed to deserialize",
    "Failed to parse the request body as JSON",
    "Failed to buffer the request body",
    "Expected request with `Content-Type: application/json`",
    "Form requests must have `Content-Type",
    "Request body didn't contain valid UTF-8",
    "Invalid URL",
    "Wrong number of path arguments",
    "No paths parameters found",
    "Missing request extension",
    "No host found in request",
    "No matched path found",
    "The matched route is not nested",
    "Connection header did not include",
    "`Upgrade` header did not include",
    "`Sec-WebSocket-",
];

/// The code a status gives an error body that has none.
pub fn status_code(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        409 => "conflict",
        413 => "too_large",
        415 => "unsupported_media",
        422 => "unprocessable",
        429 => "rate_limited",
        500..=599 => "server_error",
        _ => "bad_request",
    }
}

/// A plain sentence for a status, when the body says nothing a person can read.
fn status_sentence(status: StatusCode) -> &'static str {
    match status.as_u16() {
        401 => "kwant needs a valid sign-in for that.",
        403 => "That isn't allowed.",
        404 => "kwant couldn't find that.",
        405 => "That action isn't available here.",
        409 => "That clashes with how things are now.",
        413 => "That's too large for kwant to accept.",
        415 => "kwant can't read that kind of request.",
        422 => "That request's values weren't what kwant expected.",
        429 => "Too many requests at once. Try again in a moment.",
        500..=599 => "Something went wrong inside kwant.",
        _ => "That request wasn't in the form kwant expected.",
    }
}

/// An axum rejection's code and sentence, by its status and text.
fn rejection(status: StatusCode, text: &str) -> (&'static str, &'static str) {
    match status.as_u16() {
        413 => ("too_large", status_sentence(status)),
        415 => ("unsupported_media", "kwant expected this request as JSON (Content-Type: application/json)."),
        422 => ("unprocessable", status_sentence(status)),
        400 if text.starts_with("Failed to parse the request body as JSON") => {
            ("bad_json", "That request's body isn't valid JSON.")
        }
        500..=599 => ("server_error", status_sentence(status)),
        _ => ("bad_request", "That request wasn't in the form kwant expected."),
    }
}

fn is_rejection(text: &str) -> bool {
    REJECTION_PREFIXES.iter().any(|p| text.starts_with(p))
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

enum Kind {
    Json,
    Text,
    Empty,
}

/// What the layer may rewrite, judged from the headers alone (the body is read only after).
fn kind(resp: &Response) -> Option<Kind> {
    let status = resp.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return None;
    }
    let declared = resp.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let known = declared.or_else(|| resp.body().size_hint().exact()).or_else(|| resp.body().size_hint().upper());
    match known {
        Some(n) if n <= MAX_BODY => {}
        _ => return None,
    }
    let ct = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.to_ascii_lowercase());
    match ct.as_deref() {
        None if known == Some(0) => Some(Kind::Empty),
        None => None,
        Some(ct) => {
            let mime = ct.split(';').next().unwrap_or("").trim();
            if mime == "application/json" || mime.ends_with("+json") {
                Some(Kind::Json)
            } else if mime == "text/plain" {
                Some(Kind::Text)
            } else {
                None
            }
        }
    }
}

/// The new body for `bytes`, or `None` to send the original as it was.
fn reshape(status: StatusCode, kind: &Kind, bytes: &[u8]) -> Option<Value> {
    match kind {
        Kind::Json => {
            let Ok(Value::Object(mut obj)) = serde_json::from_slice::<Value>(bytes) else { return None };
            if obj.contains_key("code") || obj.contains_key("jsonrpc") || !obj.get("error").is_some_and(Value::is_string) {
                return None;
            }
            obj.insert("code".into(), Value::from(status_code(status)));
            Some(Value::Object(obj))
        }
        Kind::Text | Kind::Empty => {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text.is_empty() {
                return Some(json!({ "error": status_sentence(status), "code": status_code(status) }));
            }
            if is_rejection(text) {
                let (code, sentence) = rejection(status, text);
                return Some(json!({ "error": sentence, "code": code, "detail": cut(text, DETAIL_CHARS) }));
            }
            let mut obj = Map::new();
            obj.insert("error".into(), Value::from(text));
            obj.insert("code".into(), Value::from(status_code(status)));
            Some(Value::Object(obj))
        }
    }
}

/// The layer: see the module docs.
pub async fn layer(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let resp = next.run(req).await;
    if !api {
        return resp;
    }
    shape(resp).await
}

/// Gives one response the `{error, code}` shape if it needs it (public for the tests).
pub async fn shape(resp: Response) -> Response {
    let Some(kind) = kind(&resp) else { return resp };
    let (mut parts, body) = resp.into_parts();
    let bytes = match to_bytes(body, MAX_BODY as usize).await {
        Ok(b) => b,
        Err(e) => {
            // The body broke while it was read: the original can't be sent any more
            tracing::warn!(error = %e, "error body could not be read");
            parts.headers.remove(header::CONTENT_LENGTH);
            let v = json!({ "error": status_sentence(parts.status), "code": status_code(parts.status) });
            return json_response(parts, &v);
        }
    };
    match reshape(parts.status, &kind, &bytes) {
        Some(v) => json_response(parts, &v),
        None => Response::from_parts(parts, Body::from(bytes)),
    }
}

fn json_response(mut parts: axum::http::response::Parts, v: &Value) -> Response {
    let body = serde_json::to_vec(v).unwrap_or_default();
    parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    parts.headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    Response::from_parts(parts, Body::from(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Query,
        routing::{get, post},
        Json, Router,
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Q {
        page: usize,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct B {
        name: String,
    }

    fn app() -> Router {
        Router::new()
            .route("/api/q", get(|Query(_q): Query<Q>| async { "ok" }))
            .route("/api/j", post(|Json(_b): Json<B>| async { "ok" }))
            .route("/api/no-code", get(|| async { (StatusCode::NOT_FOUND, Json(json!({"error": "bot not found", "n": 2}))) }))
            .route(
                "/api/has-code",
                get(|| async { (StatusCode::CONFLICT, Json(json!({"error": "already there", "code": "exists"}))) }),
            )
            .route("/api/ok", get(|| async { Json(json!({"error": "a field called error on a 200"})) }))
            .route("/api/own-text", get(|| async { (StatusCode::FORBIDDEN, "Ana isn't in this chat.".to_string()) }))
            .route("/api/empty", get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
            .route("/api/big", get(|| async { (StatusCode::BAD_REQUEST, "x".repeat(MAX_BODY as usize + 1)) }))
            .route(
                "/api/stream",
                get(|| async {
                    let chunks = futures::stream::iter([Ok::<_, std::io::Error>("Failed to deserialize")]);
                    (StatusCode::BAD_REQUEST, [(header::CONTENT_TYPE, "text/plain")], Body::from_stream(chunks))
                }),
            )
            .route(
                "/api/download",
                get(|| async { (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/html")], "<p>nope</p>") }),
            )
            .route("/api/rpc", get(|| async { (StatusCode::BAD_REQUEST, Json(json!({"jsonrpc": "2.0", "error": "x"}))) }))
            .route("/plain", get(|| async { (StatusCode::NOT_FOUND, "404 Not Found") }))
            .layer(axum::middleware::from_fn(layer))
    }

    async fn call(method: &str, uri: &str, ct: Option<&str>, body: &str) -> (StatusCode, Option<String>, Vec<u8>) {
        let mut req = axum::http::Request::builder().method(method).uri(uri);
        if let Some(ct) = ct {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        let resp = app().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = resp.status();
        let ct = resp.headers().get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_string());
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        (status, ct, bytes)
    }

    fn json_of(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(b)))
    }

    #[tokio::test]
    async fn a_plain_text_query_rejection_becomes_a_sentence_with_a_code() {
        let (s, ct, b) = call("GET", "/api/q?page=abc", None, "").await;
        assert_eq!((s, ct.as_deref()), (StatusCode::BAD_REQUEST, Some("application/json")));
        let v = json_of(&b);
        assert_eq!(v["code"], "bad_request");
        assert_eq!(v["error"], "That request wasn't in the form kwant expected.");
        assert!(v["detail"].as_str().unwrap().starts_with("Failed to deserialize query string"), "{v}");
    }

    #[tokio::test]
    async fn json_body_rejections_get_their_own_codes() {
        let (s, _, b) = call("POST", "/api/j", Some("application/json"), "{not json").await;
        let v = json_of(&b);
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_json")), "{v}");
        assert!(v["detail"].as_str().unwrap().starts_with("Failed to parse the request body as JSON"), "{v}");

        let (s, _, b) = call("POST", "/api/j", Some("application/json"), r#"{"name": 3}"#).await;
        let v = json_of(&b);
        assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("unprocessable")), "{v}");

        let (s, _, b) = call("POST", "/api/j", Some("text/plain"), r#"{"name": "a"}"#).await;
        let v = json_of(&b);
        assert_eq!((s, v["code"].as_str()), (StatusCode::UNSUPPORTED_MEDIA_TYPE, Some("unsupported_media")), "{v}");
    }

    #[tokio::test]
    async fn a_long_parser_text_is_cut_in_detail() {
        let v = reshape(StatusCode::BAD_REQUEST, &Kind::Text, format!("Failed to deserialize {}", "é".repeat(500)).as_bytes()).unwrap();
        assert_eq!(v["detail"].as_str().unwrap().chars().count(), DETAIL_CHARS + 1);
        assert!(v["detail"].as_str().unwrap().ends_with('…'));
    }

    #[tokio::test]
    async fn json_without_a_code_gets_the_status_code_and_keeps_every_field() {
        let (s, ct, b) = call("GET", "/api/no-code", None, "").await;
        assert_eq!((s, ct.as_deref()), (StatusCode::NOT_FOUND, Some("application/json")));
        assert_eq!(json_of(&b), json!({"error": "bot not found", "n": 2, "code": "not_found"}));
    }

    #[tokio::test]
    async fn json_with_a_code_is_sent_byte_for_byte() {
        let (s, _, b) = call("GET", "/api/has-code", None, "").await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(b, serde_json::to_vec(&json!({"error": "already there", "code": "exists"})).unwrap());
    }

    #[tokio::test]
    async fn success_is_untouched() {
        let (s, _, b) = call("GET", "/api/ok", None, "").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(json_of(&b), json!({"error": "a field called error on a 200"}));
    }

    #[tokio::test]
    async fn a_handlers_own_text_stays_the_error_and_empty_bodies_get_a_sentence() {
        let (_, _, b) = call("GET", "/api/own-text", None, "").await;
        assert_eq!(json_of(&b), json!({"error": "Ana isn't in this chat.", "code": "forbidden"}));
        let (s, _, b) = call("GET", "/api/empty", None, "").await;
        assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(json_of(&b), json!({"error": "That action isn't available here.", "code": "method_not_allowed"}));
    }

    #[tokio::test]
    async fn large_streamed_and_other_bodies_pass_through() {
        let (s, ct, b) = call("GET", "/api/big", None, "").await;
        assert_eq!((s, ct.as_deref(), b.len()), (StatusCode::BAD_REQUEST, Some("text/plain; charset=utf-8"), MAX_BODY as usize + 1));
        let (s, ct, b) = call("GET", "/api/stream", None, "").await;
        assert_eq!((s, ct.as_deref(), b.as_slice()), (StatusCode::BAD_REQUEST, Some("text/plain"), b"Failed to deserialize".as_slice()));
        let (_, ct, b) = call("GET", "/api/download", None, "").await;
        assert_eq!((ct.as_deref(), b.as_slice()), (Some("text/html"), b"<p>nope</p>".as_slice()));
        let (_, _, b) = call("GET", "/api/rpc", None, "").await;
        assert_eq!(json_of(&b), json!({"jsonrpc": "2.0", "error": "x"}));
        // Outside /api/ nothing changes
        let (_, _, b) = call("GET", "/plain", None, "").await;
        assert_eq!(b, b"404 Not Found");
    }

    #[test]
    fn every_status_has_a_code() {
        for (s, c) in [
            (400, "bad_request"),
            (401, "unauthorized"),
            (403, "forbidden"),
            (404, "not_found"),
            (409, "conflict"),
            (413, "too_large"),
            (429, "rate_limited"),
            (500, "server_error"),
            (503, "server_error"),
            (418, "bad_request"),
        ] {
            assert_eq!(status_code(StatusCode::from_u16(s).unwrap()), c, "{s}");
        }
    }
}
