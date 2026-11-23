//! The source check (pa3, pa3/notes.md item 5): every answer is quoted from a page, and kwant
//! fetches that page and checks it before the quiz can open.
//!
//! For each question:
//! - the source must be `https` (no other scheme), and every address its name resolves to must be
//!   public (no loopback, LAN, link-local or metadata addresses). The fetch is pinned to the
//!   addresses that were checked; redirects are followed by hand, at most [`Policy::max_redirects`],
//!   and each new address is checked the same way;
//! - the page must answer within [`Policy::timeout`] and be at most [`Policy::max_bytes`] (the
//!   body is read in a stream and stopped at the cap), as HTML or plain text;
//! - the quote, normalised like guest answers (`matching::normalise`), must be on the page;
//! - the answer must be in the quote: the text answer or an accepted spelling as whole words, the
//!   right option's text, or for a number the same number (commas between digits ignored).
//!
//! A question that fails is listed with the reason, and the bot replaces it.

use crate::kits::quiz::matching::normalise;
use crate::kits::quiz::spec::{Kind, Question, QuizSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const USER_AGENT: &str = "kwant-quizmaster/1 (source check)";
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 3;
/// Pages checked at the same time.
pub const PARALLEL: usize = 6;

static LOCAL_HTTP: AtomicBool = AtomicBool::new(false);

/// Lets [`Policy::current`] fetch `http://` on loopback, for integration tests that serve sources
/// from a stub server on 127.0.0.1. Nothing in kwant itself calls it.
#[doc(hidden)]
pub fn allow_local_http_for_tests() {
    LOCAL_HTTP.store(true, Ordering::SeqCst);
}

/// How sources are fetched.
#[derive(Debug, Clone)]
pub struct Policy {
    pub timeout: Duration,
    pub max_bytes: usize,
    pub max_redirects: usize,
    /// Tests only: also fetch `http://` and loopback addresses (still nothing else private).
    pub local_http: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self { timeout: TIMEOUT, max_bytes: MAX_BYTES, max_redirects: MAX_REDIRECTS, local_http: false }
    }
}

impl Policy {
    /// The policy the bot tools use: the default, unless a test opened loopback.
    pub fn current() -> Self {
        Self { local_http: LOCAL_HTTP.load(Ordering::SeqCst), ..Self::default() }
    }
}

/// Why a source didn't pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fail {
    NoSource,
    BadUrl,
    NotHttps,
    PrivateAddress,
    FetchFailed,
    TooBig,
    NotAPage,
    QuoteMissing,
    AnswerNotInQuote,
}

impl Fail {
    /// The line the card shows.
    pub fn says(self) -> &'static str {
        match self {
            Fail::NoSource => "no source",
            Fail::BadUrl => "the source address can't be read",
            Fail::NotHttps => "the source isn't https",
            Fail::PrivateAddress => "the source is on a private or local address",
            Fail::FetchFailed => "the source page didn't load",
            Fail::TooBig => "the source page is bigger than 2 MB",
            Fail::NotAPage => "the source isn't a web page",
            Fail::QuoteMissing => "the quote isn't on that page",
            Fail::AnswerNotInQuote => "the quote doesn't contain the answer",
        }
    }
}

/// One question's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceResult {
    /// Flat 0-based question index.
    pub q: usize,
    /// "round 1, question 2".
    pub at: String,
    pub url: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail: Option<Fail>,
    /// More about a failure (an HTTP status, the address refused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// What was checked: url, quote and answer. An unchanged key reuses the result.
    pub key: String,
}

/// The cache key: a question's source and answer fields.
pub fn key(q: &Question) -> String {
    let (url, quote) = q.source.as_ref().map(|s| (s.url.as_str(), s.quote.as_str())).unwrap_or_default();
    let parts = serde_json::json!([url, quote, q.kind.map(Kind::as_str), q.answer, q.accept, q.options, q.tolerance]);
    let digest = Sha256::digest(parts.to_string().as_bytes());
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// pa4c: a person's own question's key: what they wrote (text, kind, options, answer and
/// spellings). The draft keeps these (`quiz_drafts.own_keys`), so only questions the person
/// wrote skip the source check; a bot writing `{"own": true}` gets "no source" as before.
pub fn own_key(q: &Question) -> String {
    let parts = serde_json::json!(["own", q.text.trim(), q.kind.map(Kind::as_str), q.options, q.answer, q.accept, q.tolerance]);
    let digest = Sha256::digest(parts.to_string().as_bytes());
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Whether a question is one of the person's own (see [`own_key`]).
pub fn is_own(q: &Question, own: &HashSet<String>) -> bool {
    q.source.as_ref().is_some_and(|s| s.own) && own.contains(&own_key(q))
}

/// Every question with where it sits.
pub fn questions(spec: &QuizSpec) -> Vec<(String, &Question)> {
    let mut out = Vec::new();
    for (r, round) in spec.rounds.iter().enumerate() {
        for (i, q) in round.questions.iter().enumerate() {
            out.push((format!("round {}, question {}", r + 1, i + 1), q));
        }
    }
    out
}

/// Checks every question's source. A question whose [`key`] is in `cache` keeps that result.
pub async fn check_spec(spec: &QuizSpec, policy: &Policy, cache: &HashMap<String, SourceResult>) -> Vec<SourceResult> {
    check_spec_skipping(spec, policy, cache, &HashSet::new()).await
}

/// [`check_spec`] leaving out the person's own questions ([`is_own`]): they have no result.
pub async fn check_spec_skipping(spec: &QuizSpec, policy: &Policy, cache: &HashMap<String, SourceResult>, own: &HashSet<String>) -> Vec<SourceResult> {
    use futures::stream::StreamExt;
    let items: Vec<(usize, String, Question)> =
        questions(spec).into_iter().enumerate().filter(|(_, (_, q))| !is_own(q, own)).map(|(n, (at, q))| (n, at, q.clone())).collect();
    let mut out: Vec<SourceResult> = futures::stream::iter(items)
        .map(|(n, at, q)| async move {
            let k = key(&q);
            if let Some(hit) = cache.get(&k) {
                return SourceResult { q: n, at, ..hit.clone() };
            }
            check_question(n, at, &q, policy).await
        })
        .buffer_unordered(PARALLEL)
        .collect()
        .await;
    out.sort_by_key(|r| r.q);
    out
}

/// Checks one question's source.
pub async fn check_question(n: usize, at: String, q: &Question, policy: &Policy) -> SourceResult {
    let k = key(q);
    let url = q.source.as_ref().map(|s| s.url.trim().to_string()).unwrap_or_default();
    let done = |fail: Option<Fail>, detail: Option<String>| SourceResult { q: n, at: at.clone(), url: url.clone(), ok: fail.is_none(), fail, detail, key: k.clone() };
    let Some(src) = q.source.as_ref().filter(|s| !s.url.trim().is_empty() && !s.quote.trim().is_empty()) else {
        return done(Some(Fail::NoSource), None);
    };
    if !answer_in_quote(q, &src.quote) {
        return done(Some(Fail::AnswerNotInQuote), None);
    }
    let page = match fetch_text(&url, policy).await {
        Ok(p) => p,
        Err((fail, detail)) => return done(Some(fail), detail),
    };
    if !quote_on_page(&src.quote, &page) {
        return done(Some(Fail::QuoteMissing), None);
    }
    done(None, None)
}

/// Removes citation markers such as `[12]` or `[note 3]`, which pages show and quotes leave out.
fn strip_citations(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        match after.find(']') {
            Some(j) if j <= 16 && after[1..j].chars().all(|c| c.is_alphanumeric() || c == ' ') => {
                out.push(' ');
                rest = &after[j + 1..];
            }
            _ => {
                out.push('[');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// True when the normalised quote is on the normalised page, as whole words.
pub fn quote_on_page(quote: &str, page: &str) -> bool {
    let q = normalise(&strip_citations(quote));
    if q.is_empty() {
        return false;
    }
    let p = normalise(&strip_citations(page));
    format!(" {p} ").contains(&format!(" {q} "))
}

fn has_words(haystack: &str, needle: &str) -> bool {
    let n = normalise(needle);
    !n.is_empty() && format!(" {haystack} ").contains(&format!(" {n} "))
}

/// The numbers written in a text ("1,989", "9.5", "-3").
fn numbers_in(s: &str) -> Vec<f64> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() || (i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '.')) {
            i += 1;
            continue;
        }
        let neg = i > 0 && chars[i - 1] == '-' && (i < 2 || !chars[i - 2].is_alphanumeric());
        let mut num = String::new();
        let mut j = i;
        while j < chars.len() {
            let c = chars[j];
            let next_digit = chars.get(j + 1).is_some_and(|d| d.is_ascii_digit());
            if c.is_ascii_digit() {
                num.push(c);
            } else if c == ',' && next_digit {
                // thousands separator
            } else if c == '.' && next_digit && !num.contains('.') {
                num.push('.');
            } else {
                break;
            }
            j += 1;
        }
        if chars.get(j).is_none_or(|c| !c.is_alphabetic()) {
            if let Ok(v) = num.parse::<f64>() {
                out.push(if neg { -v } else { v });
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// True when the quote contains the question's answer.
pub fn answer_in_quote(q: &Question, quote: &str) -> bool {
    let nq = normalise(&strip_citations(quote));
    let is_number = q.kind == Some(Kind::Number) || (q.kind.is_none() && q.answer_number().is_some());
    if is_number {
        let Some(want) = q.answer_number() else { return false };
        let tol = q.tolerance().abs() + 1e-9;
        return numbers_in(quote).iter().any(|n| (n - want).abs() <= tol);
    }
    let answer = q.answer_text();
    if q.kind == Some(Kind::Choice) {
        return has_words(&nq, answer);
    }
    std::iter::once(answer).chain(q.accepted().iter().map(String::as_str)).any(|a| has_words(&nq, a))
}

/// The text of an HTML page: no scripts, styles or comments, tags as spaces, entities decoded.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let rest = &lower[i..];
            let skip_to = if rest.starts_with("<!--") {
                rest.find("-->").map(|e| i + e + 3)
            } else if rest.starts_with("<script") || rest.starts_with("<style") || rest.starts_with("<noscript") || rest.starts_with("<template") {
                let tag = if rest.starts_with("<script") {
                    "</script"
                } else if rest.starts_with("<style") {
                    "</style"
                } else if rest.starts_with("<noscript") {
                    "</noscript"
                } else {
                    "</template"
                };
                rest.find(tag).and_then(|e| lower[i + e..].find('>').map(|g| i + e + g + 1))
            } else {
                rest.find('>').map(|e| i + e + 1)
            };
            out.push(' ');
            i = skip_to.unwrap_or(b.len());
            continue;
        }
        let next = html[i..].find('<').map(|n| i + n).unwrap_or(b.len());
        out.push_str(&decode_entities(&html[i..next]));
        i = next;
    }
    out
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &after[..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" | "#160" => Some(' '),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "rsquo" | "lsquo" => Some('\''),
                "rdquo" | "ldquo" => Some('"'),
                n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

type FetchErr = (Fail, Option<String>);

/// Checks a URL's scheme and addresses; returns the host name and the addresses to pin.
async fn allowed(url: &url::Url, policy: &Policy) -> Result<(String, Vec<SocketAddr>), FetchErr> {
    let local = policy.local_http && url.host_str().is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]");
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        other => return Err((Fail::NotHttps, Some(format!("it is a `{other}:` address")))),
    }
    if local {
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(80);
        let ip: std::net::IpAddr = if host == "localhost" { std::net::Ipv4Addr::LOCALHOST.into() } else { host.trim_matches(['[', ']']).parse().map_err(|_| (Fail::BadUrl, None))? };
        return Ok((host, vec![SocketAddr::new(ip, port)]));
    }
    let text = url.to_string();
    let checked = tokio::task::spawn_blocking(move || crate::mcp::fetch_risk::public_only(&text))
        .await
        .map_err(|_| (Fail::FetchFailed, None))?;
    checked.map_err(|why| (Fail::PrivateAddress, Some(why)))
}

/// Fetches a page's text under the policy.
pub async fn fetch_text(url: &str, policy: &Policy) -> Result<String, FetchErr> {
    match tokio::time::timeout(policy.timeout, fetch_inner(url, policy)).await {
        Ok(r) => r,
        Err(_) => Err((Fail::FetchFailed, Some(format!("no answer within {} s", policy.timeout.as_secs())))),
    }
}

async fn fetch_inner(url: &str, policy: &Policy) -> Result<String, FetchErr> {
    let mut at = url::Url::parse(url.trim()).map_err(|_| (Fail::BadUrl, None))?;
    for _ in 0..=policy.max_redirects {
        let (host, addrs) = allowed(&at, policy).await?;
        let client = reqwest::Client::builder()
            .resolve_to_addrs(&host, &addrs)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(policy.timeout)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| (Fail::FetchFailed, None))?;
        let mut resp = client
            .get(at.clone())
            .header(reqwest::header::ACCEPT, "text/html, text/plain;q=0.9")
            .send()
            .await
            .map_err(|e| (Fail::FetchFailed, Some(if e.is_timeout() { "it timed out".into() } else { "it couldn't be reached".into() })))?;
        let status = resp.status();
        if status.is_redirection() {
            let to = resp.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok()).and_then(|l| at.join(l).ok());
            at = to.ok_or((Fail::FetchFailed, Some(format!("it answered {} without a new address", status.as_u16()))))?;
            continue;
        }
        if !status.is_success() {
            return Err((Fail::FetchFailed, Some(format!("it answered {}", status.as_u16()))));
        }
        if resp.content_length().is_some_and(|n| n as usize > policy.max_bytes) {
            return Err((Fail::TooBig, None));
        }
        let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|c| c.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let html = ctype.contains("html");
        if !html && !ctype.starts_with("text/plain") {
            return Err((Fail::NotAPage, Some(if ctype.is_empty() { "no content type".into() } else { ctype })));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|_| (Fail::FetchFailed, Some("the page stopped loading".into())))? {
            body.extend_from_slice(&chunk);
            if body.len() > policy.max_bytes {
                return Err((Fail::TooBig, None));
            }
        }
        let text = String::from_utf8_lossy(&body);
        return Ok(if html { html_to_text(&text) } else { text.into_owned() });
    }
    Err((Fail::FetchFailed, Some(format!("more than {} redirects", policy.max_redirects))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::quiz::spec::Source;
    use axum::{http::header, response::IntoResponse, routing::get, Router};

    fn text_q(answer: &str, accept: &[&str], url: &str, quote: &str) -> Question {
        Question {
            kind: Some(Kind::Text),
            text: "?".into(),
            answer: Some(answer.into()),
            accept: (!accept.is_empty()).then(|| accept.iter().map(|s| s.to_string()).collect()),
            source: Some(Source { url: url.into(), quote: quote.into(), own: false }),
            ..Default::default()
        }
    }

    fn number_q(answer: f64, quote: &str) -> Question {
        Question {
            kind: Some(Kind::Number),
            text: "?".into(),
            answer: Some(serde_json::json!(answer)),
            source: Some(Source { url: "https://example.org".into(), quote: quote.into(), own: false }),
            ..Default::default()
        }
    }

    #[test]
    fn answer_must_be_in_the_quote_as_whole_words() {
        let q = text_q("Paris", &[], "", "The Eiffel Tower is a wrought-iron lattice tower in Paris, France.");
        assert!(answer_in_quote(&q, &q.source.as_ref().unwrap().quote));
        assert!(!answer_in_quote(&q, "It stands in Parisville."));
        assert!(!answer_in_quote(&q, "It stands in Lyon."));
        let z = text_q("Zinedine Zidane", &["Zidane"], "", "Zidane won the award");
        assert!(answer_in_quote(&z, "Zidane won the award"), "an accepted spelling counts");
        let choice = Question { kind: Some(Kind::Choice), options: Some(vec!["Blur".into(), "Oasis".into()]), answer: Some("Oasis".into()), ..Default::default() };
        assert!(answer_in_quote(&choice, "the debut studio album by English rock band Oasis, released"));
        assert!(!answer_in_quote(&choice, "the debut studio album by Blur"));
    }

    #[test]
    fn numbers_match_exactly_or_within_tolerance() {
        assert!(answer_in_quote(&number_q(1989.0, "on 9 November 1989"), "on 9 November 1989"));
        assert!(answer_in_quote(&number_q(8849.0, "It is 8,849 metres tall."), "It is 8,849 metres tall."));
        assert!(!answer_in_quote(&number_q(1989.0, "in 19890 or so"), "in 19890 or so"));
        assert!(!answer_in_quote(&number_q(1989.0, "x"), "the year 1988"));
        assert!(!answer_in_quote(&number_q(9.0, "x"), "on 9th"), "a number glued to letters isn't the number");
        let mut q = number_q(100.0, "x");
        q.tolerance = Some(2.0);
        assert!(answer_in_quote(&q, "about 101 people"));
        assert_eq!(numbers_in("about 2.75, -2 and 1,000,000"), vec![2.75, -2.0, 1_000_000.0]);
    }

    #[test]
    fn quote_on_page_ignores_case_markup_and_citations() {
        let page = html_to_text(
            "<html><head><style>p{}</style><script>var q='Paris';</script></head><body><p>The <b>Eiffel&nbsp;Tower</b> is in \
             Paris,[12] France&#46;</p><!-- the Louvre --></body></html>",
        );
        assert!(quote_on_page("the Eiffel Tower is in Paris, France.", &page), "{page}");
        assert!(!quote_on_page("var q", &page), "scripts are not text");
        assert!(!quote_on_page("the Louvre", &page), "comments are not text");
        assert!(!quote_on_page("Eiffel Tower is in Lyon", &page));
        assert!(!quote_on_page("Tower is in Par", &page), "whole words only");
        assert!(!quote_on_page("   ", &page));
    }

    #[test]
    fn keys_change_with_the_answer_or_source() {
        let a = text_q("Paris", &[], "https://a.example/x", "in Paris");
        let mut b = a.clone();
        assert_eq!(key(&a), key(&b));
        b.answer = Some("Lyon".into());
        assert_ne!(key(&a), key(&b));
        let mut c = a.clone();
        c.source.as_mut().unwrap().quote = "in Paris, France".into();
        assert_ne!(key(&a), key(&c));
        let mut d = a.clone();
        d.text = "A reworded question".into();
        assert_eq!(key(&a), key(&d), "rewording the question keeps the check");
    }

    async fn stub() -> (String, tokio::task::JoinHandle<()>) {
        let big = "a ".repeat(MAX_BYTES);
        let app = Router::new()
            .route("/eiffel", get(|| async { ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], "<p>The Eiffel Tower is a wrought-iron lattice tower on the Champ de Mars in Paris, France.</p>") }))
            .route("/plain", get(|| async { ([(header::CONTENT_TYPE, "text/plain")], "Paris is the capital of France.") }))
            .route("/big", get(move || async move { ([(header::CONTENT_TYPE, "text/html")], big).into_response() }))
            .route("/pdf", get(|| async { ([(header::CONTENT_TYPE, "application/pdf")], "%PDF Paris") }))
            .route("/missing", get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }))
            .route("/hop", get(|| async { (axum::http::StatusCode::FOUND, [(header::LOCATION, "/eiffel")]) }))
            .route("/to-lan", get(|| async { (axum::http::StatusCode::FOUND, [(header::LOCATION, "http://10.0.0.7/eiffel")]) }))
            .route("/loop", get(|| async { (axum::http::StatusCode::FOUND, [(header::LOCATION, "/loop")]) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let h = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (base, h)
    }

    fn local() -> Policy {
        Policy { local_http: true, ..Policy::default() }
    }

    const QUOTE: &str = "The Eiffel Tower is a wrought-iron lattice tower on the Champ de Mars in Paris, France.";

    async fn run(q: &Question, policy: &Policy) -> SourceResult {
        check_question(0, "round 1, question 1".into(), q, policy).await
    }

    #[tokio::test]
    async fn stub_server_quote_present_missing_and_answer_not_implied() {
        let (base, h) = stub().await;
        let ok = run(&text_q("Paris", &[], &format!("{base}/eiffel"), QUOTE), &local()).await;
        assert!(ok.ok, "{ok:?}");
        let via_redirect = run(&text_q("Paris", &[], &format!("{base}/hop"), QUOTE), &local()).await;
        assert!(via_redirect.ok, "{via_redirect:?}");
        let plain = run(&text_q("Paris", &[], &format!("{base}/plain"), "Paris is the capital of France"), &local()).await;
        assert!(plain.ok, "{plain:?}");

        let missing = run(&text_q("Paris", &[], &format!("{base}/eiffel"), "The Eiffel Tower is in Paris, the capital of France."), &local()).await;
        assert_eq!(missing.fail, Some(Fail::QuoteMissing));

        let not_implied = run(&text_q("Lyon", &[], &format!("{base}/eiffel"), QUOTE), &local()).await;
        assert_eq!(not_implied.fail, Some(Fail::AnswerNotInQuote));
        h.abort();
    }

    #[tokio::test]
    async fn stub_server_too_big_not_a_page_and_errors() {
        let (base, h) = stub().await;
        let big = run(&text_q("a", &[], &format!("{base}/big"), "a a"), &local()).await;
        assert_eq!(big.fail, Some(Fail::TooBig), "{big:?}");
        let pdf = run(&text_q("Paris", &[], &format!("{base}/pdf"), "PDF Paris"), &local()).await;
        assert_eq!(pdf.fail, Some(Fail::NotAPage));
        let gone = run(&text_q("Paris", &[], &format!("{base}/missing"), QUOTE), &local()).await;
        assert_eq!(gone.fail, Some(Fail::FetchFailed));
        assert_eq!(gone.detail.as_deref(), Some("it answered 404"));
        let looped = run(&text_q("Paris", &[], &format!("{base}/loop"), QUOTE), &local()).await;
        assert_eq!(looped.fail, Some(Fail::FetchFailed));
        let to_lan = run(&text_q("Paris", &[], &format!("{base}/to-lan"), QUOTE), &local()).await;
        assert_eq!(to_lan.fail, Some(Fail::NotHttps), "a redirect is checked again: {to_lan:?}");
        h.abort();
    }

    #[tokio::test]
    async fn the_real_policy_refuses_http_and_private_addresses() {
        let (base, h) = stub().await;
        let strict = Policy::default();
        let http = run(&text_q("Paris", &[], &format!("{base}/eiffel"), QUOTE), &strict).await;
        assert_eq!(http.fail, Some(Fail::NotHttps), "{http:?}");
        for url in [
            "https://127.0.0.1/eiffel",
            "https://localhost/eiffel",
            "https://10.0.0.7/eiffel",
            "https://192.168.1.1/eiffel",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/eiffel",
            "https://[fd00::1]/eiffel",
        ] {
            let r = run(&text_q("Paris", &[], url, QUOTE), &strict).await;
            assert_eq!(r.fail, Some(Fail::PrivateAddress), "{url}: {r:?}");
        }
        // Even with loopback opened for tests, other private addresses stay shut
        let lan = run(&text_q("Paris", &[], "https://10.0.0.7/eiffel", QUOTE), &local()).await;
        assert_eq!(lan.fail, Some(Fail::PrivateAddress));
        for url in ["ftp://example.org/x", "file:///etc/passwd", "javascript:alert(1)"] {
            let r = run(&text_q("Paris", &[], url, QUOTE), &strict).await;
            assert!(matches!(r.fail, Some(Fail::NotHttps) | Some(Fail::BadUrl)), "{url}: {r:?}");
        }
        let none = run(&Question { kind: Some(Kind::Text), answer: Some("Paris".into()), ..Default::default() }, &strict).await;
        assert_eq!(none.fail, Some(Fail::NoSource));
        h.abort();
    }

    #[tokio::test]
    async fn cached_results_skip_the_fetch() {
        let spec: QuizSpec = serde_json::from_str(crate::kits::quiz::EXAMPLE).unwrap();
        let qs = questions(&spec);
        let mut cache = HashMap::new();
        for (n, (at, q)) in qs.iter().enumerate() {
            cache.insert(key(q), SourceResult { q: n, at: at.clone(), url: "cached".into(), ok: true, fail: None, detail: None, key: key(q) });
        }
        let out = check_spec(&spec, &Policy::default(), &cache).await;
        assert_eq!(out.len(), qs.len());
        assert!(out.iter().all(|r| r.ok && r.url == "cached"));
        assert_eq!(out.iter().map(|r| r.q).collect::<Vec<_>>(), (0..qs.len()).collect::<Vec<_>>());
    }
}
