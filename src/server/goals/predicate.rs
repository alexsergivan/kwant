//! A goal check's typed predicate (docs/mission-control.md §22): which value of a run it reads
//! (the metric), how it compares (the operator) and with what (the target). The file kinds keep
//! their v1 meaning and take none. Matching is literal: no regex, no shell, nothing in a target is
//! evaluated. An error, or output the predicate can't read, is `unknown`, never `not_met`.

use crate::server::missions::autofix::redact;
use crate::server::missions::checks::{run_spec_seen, Seen};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

pub const MET: &str = "met";
pub const NOT_MET: &str = "not_met";
pub const UNKNOWN: &str = "unknown";

/// Largest number a numeric target may hold.
pub const MAX_NUMBER: f64 = 1e15;
pub const MAX_TARGET_CHARS: usize = 1000;
pub const MAX_LATENCY_MS: f64 = 600_000.0;
/// Longest stored observed value.
const MAX_OBSERVED_CHARS: usize = 500;

/// The metrics `kind` offers; none for the file kinds.
pub fn metrics(kind: &str) -> &'static [&'static str] {
    match kind {
        "http" => &["status", "latency_ms", "body_contains", "json_value"],
        "command" => &["exit_code", "stdout_contains", "stdout_number"],
        _ => &[],
    }
}

/// Every operator some metric takes, for the `goal_propose` schema. `contains` is a json_value
/// operator; the contains metrics (body_contains, stdout_contains) accept it too and ignore it.
pub const ALL_OPERATORS: [&str; 7] = ["eq", "ne", "lt", "lte", "gt", "gte", "contains"];

/// The operators `metric` takes, its default first. Empty: a contains metric, which matches its
/// target as written and takes no operator.
pub fn operators(metric: &str) -> &'static [&'static str] {
    match metric {
        "status" | "exit_code" => &["eq", "ne"],
        "latency_ms" => &["lt", "lte"],
        "json_value" => &["eq", "ne", "lt", "lte", "gt", "gte", "contains"],
        "stdout_number" => &["eq", "ne", "lt", "lte", "gt", "gte"],
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Predicate {
    pub metric: String,
    /// None for a contains metric.
    pub operator: Option<String>,
    pub target: String,
}

impl Predicate {
    fn new(metric: &str, operator: Option<&str>, target: &str) -> Self {
        Predicate { metric: metric.into(), operator: operator.map(Into::into), target: target.into() }
    }

    pub fn to_json(&self) -> Value {
        json!({ "metric": self.metric, "operator": self.operator, "target": self.target })
    }

    /// A predicate a fix task's check carries in `spec.predicate`.
    pub fn from_json(v: &Value) -> Option<Self> {
        let metric = v["metric"].as_str()?;
        let target = v["target"].as_str()?;
        Some(Predicate::new(metric, v["operator"].as_str(), target))
    }
}

/// What a check of `kind` means when the person gave no metric: status 200 for http, exit code 0
/// for a command; None for the file kinds.
pub fn default_predicate(kind: &str) -> Option<Predicate> {
    match kind {
        "http" => Some(Predicate::new("status", Some("eq"), "200")),
        "command" => Some(Predicate::new("exit_code", Some("eq"), "0")),
        _ => None,
    }
}

/// A finite number within [`MAX_NUMBER`], as a target or a JSON string holds it.
pub fn number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|n| n.is_finite() && n.abs() <= MAX_NUMBER)
}

fn whole(s: &str, lo: i64, hi: i64) -> Option<i64> {
    s.trim().parse::<i64>().ok().filter(|n| (lo..=hi).contains(n))
}

/// A JSON pointer: empty (the whole document) or `/`-separated, with `~0` / `~1` escapes only.
fn valid_pointer(p: &str) -> bool {
    if p.is_empty() {
        return true;
    }
    if !p.starts_with('/') || p.len() > 500 {
        return false;
    }
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

/// Checks and completes a check's predicate. With no metric, operator or target a kind gets its
/// default ([`default_predicate`]); a metric without an operator gets the metric's first; every
/// metric needs a target. Returns None for the file kinds.
pub fn normalize(kind: &str, spec: &Value, metric: Option<&str>, operator: Option<&str>, target: Option<&str>) -> Result<Option<Predicate>, String> {
    let metric = metric.map(str::trim).filter(|m| !m.is_empty());
    let operator = operator.map(str::trim).filter(|o| !o.is_empty());
    let offered = metrics(kind);
    if offered.is_empty() {
        if metric.is_some() || operator.is_some() || target.is_some() {
            return Err(format!("{kind} checks take no metric, operator or target"));
        }
        return Ok(None);
    }
    let Some(metric) = metric else {
        if operator.is_some() || target.is_some() {
            return Err(format!("say which value the target is for: metric is one of {}", offered.join(", ")));
        }
        return Ok(default_predicate(kind));
    };
    if !offered.contains(&metric) {
        return Err(format!("a {kind} check's metric is one of {}, not {metric}", offered.join(", ")));
    }
    let ops = operators(metric);
    let operator = match operator {
        None if ops.is_empty() => None,
        Some("contains") if ops.is_empty() => None,
        Some(o) if ops.is_empty() => return Err(format!("{metric} matches its target as written and takes no operator, not {o}")),
        None => Some(ops[0]),
        Some(o) if ops.contains(&o) => Some(o),
        Some(o) => return Err(format!("{metric} takes {}, not {o}", ops.join(", "))),
    };
    let target = target.ok_or_else(|| format!("{metric} needs a target"))?;
    if target.chars().count() > MAX_TARGET_CHARS {
        return Err(format!("the target is too long ({MAX_TARGET_CHARS} characters at most)"));
    }
    let target = match metric {
        "status" => whole(target, 100, 599).ok_or("an http status target is a whole number from 100 to 599")?.to_string(),
        "exit_code" => whole(target, 0, 255).ok_or("an exit code target is a whole number from 0 to 255")?.to_string(),
        "latency_ms" => {
            let n = number(target).filter(|n| (0.0..=MAX_LATENCY_MS).contains(n)).ok_or("a latency target is a number of milliseconds from 0 to 600000")?;
            // A latency is never below 0, so "under 0 ms" can never be met
            if n == 0.0 && operator == Some("lt") {
                return Err("a latency can't be under 0 ms, so that goal could never be met: use 1 or more with lt, or lte 0".into());
            }
            plain(n)
        }
        "stdout_number" => plain(number(target).ok_or("stdout_number needs a number target, at most 1e15 either way")?),
        "json_value" => {
            if !spec["pointer"].is_null() && !spec["pointer"].as_str().is_some_and(valid_pointer) {
                return Err("spec.pointer is a JSON pointer: empty for the whole document, or like /status or /items/0/name".into());
            }
            match operator {
                Some("lt" | "lte" | "gt" | "gte") => plain(number(target).ok_or("a json_value compared with lt, lte, gt or gte needs a number target, at most 1e15 either way")?),
                Some("contains") if target.is_empty() => return Err("json_value contains needs a target".into()),
                _ => target.to_string(),
            }
        }
        _ if target.is_empty() => return Err(format!("{metric} needs the text to look for")),
        _ => target.to_string(),
    };
    Ok(Some(Predicate::new(metric, operator, &target)))
}

/// A number as a target stores it: whole numbers without a decimal point.
fn plain(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// The first number in `text`: an optional `-`, digits with optional `,ddd` thousands groups, an
/// optional decimal part and an optional exponent (`Total: 1,234.5` is 1234.5, `1e3` is 1000,
/// `42abc` is 42). None if there is none.
pub fn first_number(text: &str) -> Option<f64> {
    let b = text.as_bytes();
    let digit = |k: usize| b.get(k).is_some_and(u8::is_ascii_digit);
    let mut i = 0;
    while i < b.len() {
        let neg = b[i] == b'-';
        let mut k = i + usize::from(neg);
        if !(digit(k) || (b.get(k) == Some(&b'.') && digit(k + 1))) {
            i += 1;
            continue;
        }
        let mut s = String::from(if neg { "-" } else { "" });
        while digit(k) {
            s.push(b[k] as char);
            k += 1;
        }
        while b.get(k) == Some(&b',') && digit(k + 1) && digit(k + 2) && digit(k + 3) && !digit(k + 4) {
            s.push_str(&text[k + 1..k + 4]);
            k += 4;
        }
        if b.get(k) == Some(&b'.') && digit(k + 1) {
            s.push('.');
            k += 1;
            while digit(k) {
                s.push(b[k] as char);
                k += 1;
            }
        }
        if matches!(b.get(k), Some(b'e' | b'E')) {
            let sign = matches!(b.get(k + 1), Some(b'+' | b'-'));
            let mut e = k + 1 + usize::from(sign);
            if digit(e) {
                s.push('e');
                if sign {
                    s.push(b[k + 1] as char);
                }
                while digit(e) {
                    s.push(b[e] as char);
                    e += 1;
                }
            }
        }
        return s.parse::<f64>().ok().filter(|n| n.is_finite());
    }
    None
}

fn compare(value: f64, op: &str, target: f64) -> Option<bool> {
    Some(match op {
        "eq" => value == target,
        "ne" => value != target,
        "lt" => value < target,
        "lte" => value <= target,
        "gt" => value > target,
        "gte" => value >= target,
        _ => return None,
    })
}

fn verdict(holds: Option<bool>) -> &'static str {
    match holds {
        Some(true) => MET,
        Some(false) => NOT_MET,
        None => UNKNOWN,
    }
}

fn shown(n: f64) -> String {
    plain(n)
}

/// A JSON value as text: a string as written, anything else as JSON.
fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A JSON number, or a string that reads as one (eq and ne compare leniently: 200 equals "200").
fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => number(s),
        _ => None,
    }
}

fn text_contains(haystack: Option<&str>, needle: &str) -> (&'static str, Option<String>) {
    match haystack {
        None => (UNKNOWN, None),
        Some(h) if h.contains(needle) => (MET, Some("found".into())),
        Some(_) => (NOT_MET, Some("not found".into())),
    }
}

fn json_value(body: Option<&str>, pointer: &str, op: &str, target: &str) -> (&'static str, Option<String>) {
    let Some(body) = body else { return (UNKNOWN, None) };
    let Ok(doc) = serde_json::from_str::<Value>(body) else { return (UNKNOWN, Some("not JSON".into())) };
    // A JSON answer without the value is an answer: not met
    let Some(v) = doc.pointer(pointer) else { return (NOT_MET, Some("missing".into())) };
    let holds = match op {
        "eq" | "ne" => {
            let same = match (as_number(v), number(target)) {
                (Some(a), Some(b)) => a == b,
                _ => render(v) == target,
            };
            Some(same == (op == "eq"))
        }
        "contains" => match v {
            Value::String(s) => Some(s.contains(target)),
            Value::Array(items) => Some(items.iter().any(|i| match (as_number(i), number(target)) {
                (Some(a), Some(b)) => a == b,
                _ => render(i) == target,
            })),
            _ => None,
        },
        o => match (as_number(v), number(target)) {
            (Some(a), Some(b)) => compare(a, o, b),
            _ => None,
        },
    };
    (verdict(holds), Some(render(v)))
}

/// Why a body check on a page past the read cap says nothing: the needle may be in the part that
/// wasn't read.
pub const TOO_BIG: &str = "The page is larger than 5 MB, so I can't check it";

/// The plain reason a 3xx answer is not met: kwant doesn't follow redirects (a redirect could lead
/// to an address the check wasn't approved for), so the person points the check at the final one.
fn redirect_reason(host: Option<&str>) -> String {
    match host {
        Some(h) => format!("it redirects to {h}, and kwant doesn't follow redirects: use the final address"),
        None => "it redirects to another address on the same site, and kwant doesn't follow redirects: use the final address".into(),
    }
}

/// One check's verdict on what its run saw, with the value it read. `status` is the run's own
/// result: `error` (it didn't run, or got no answer) is always unknown.
pub fn judge(pred: Option<&Predicate>, spec: &Value, status: &str, seen: &Seen, latency_ms: i64) -> (&'static str, Option<String>) {
    let (verdict, observed) = judge_value(pred, spec, status, seen, latency_ms);
    let metric = pred.map(|p| p.metric.as_str());
    // A body past the read cap: a needle that wasn't found may be in the part that wasn't read
    if seen.body_truncated && verdict != MET && matches!(metric, Some("body_contains" | "json_value")) {
        return (UNKNOWN, Some(TOO_BIG.into()));
    }
    // A 3xx answer that wasn't met: say where it points
    if verdict == NOT_MET {
        if let Some(to) = &seen.redirect {
            let why = redirect_reason(to.as_deref());
            return (verdict, Some(match observed {
                Some(o) => format!("{o}: {why}"),
                None => why,
            }));
        }
    }
    (verdict, observed)
}

fn judge_value(pred: Option<&Predicate>, spec: &Value, status: &str, seen: &Seen, latency_ms: i64) -> (&'static str, Option<String>) {
    if status == "error" {
        return (UNKNOWN, None);
    }
    let Some(p) = pred else {
        // The file kinds: v1's meaning
        return (if status == "passed" { MET } else { NOT_MET }, None);
    };
    let op = p.operator.as_deref().unwrap_or("");
    let target = number(&p.target);
    match p.metric.as_str() {
        "status" => match (seen.http_status, target) {
            (Some(code), Some(t)) => (verdict(compare(f64::from(code), op, t)), Some(code.to_string())),
            (code, _) => (UNKNOWN, code.map(|c| c.to_string())),
        },
        "latency_ms" => match target {
            Some(t) => (verdict(compare(latency_ms as f64, op, t)), Some(latency_ms.to_string())),
            None => (UNKNOWN, Some(latency_ms.to_string())),
        },
        "body_contains" => text_contains(seen.body.as_deref(), &p.target),
        "json_value" => json_value(seen.body.as_deref(), spec["pointer"].as_str().unwrap_or(""), op, &p.target),
        "exit_code" => match (seen.exit_code, target) {
            // Killed, not executable or not found: the command didn't say anything about the goal
            (Some(code), _) if code < 0 || code == 126 || code == 127 => (UNKNOWN, Some(code.to_string())),
            (Some(code), Some(t)) => (verdict(compare(code as f64, op, t)), Some(code.to_string())),
            (code, _) => (UNKNOWN, code.map(|c| c.to_string())),
        },
        "stdout_contains" => text_contains(seen.stdout.as_deref(), &p.target),
        "stdout_number" => match (seen.stdout.as_deref().map(first_number), target) {
            (Some(Some(n)), Some(t)) => (verdict(compare(n, op, t)), Some(shown(n))),
            (Some(None), _) => (UNKNOWN, Some("no number".into())),
            _ => (UNKNOWN, None),
        },
        _ => (UNKNOWN, None),
    }
}

/// A check's run, judged.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// What the check did: `passed`, `failed` or `error` (v1's meaning).
    pub status: String,
    pub verdict: &'static str,
    /// Redacted, at most 500 characters.
    pub observed: Option<String>,
    /// Redacted output, at most 2 000 characters.
    pub excerpt: String,
}

fn cap(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n - 1).collect();
    out.push('…');
    out
}

/// Runs one check and judges it. The latency is the whole GET, body included (up to 5 MB).
/// Nothing but the redacted excerpt and observed value leaves here: never the body.
pub async fn run(kind: &str, spec: &Value, pred: Option<&Predicate>, folder: Option<&Path>, person_ok: bool) -> Outcome {
    let started = Instant::now();
    let (status, output, seen) = run_spec_seen(kind, spec, folder, person_ok).await;
    let latency = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let (verdict, observed) = judge(pred, spec, &status, &seen, latency);
    let excerpt = sha256_excerpt(spec, &seen).unwrap_or_else(|| redact(&output));
    Outcome { status, verdict, observed: observed.map(|o| cap(&redact(&o), MAX_OBSERVED_CHARS)), excerpt }
}

/// What a `file_sha256` check shows: both hashes, which `redact` would hide as random-looking
/// words. They are hashes, not file contents, so the person can compare them. None unless both
/// read as hex.
fn sha256_excerpt(spec: &Value, seen: &Seen) -> Option<String> {
    let (got, want) = seen.sha256.as_ref()?;
    let hex = |h: &str| !h.is_empty() && h.len() <= 64 && h.chars().all(|c| c.is_ascii_hexdigit());
    if !hex(got) || !hex(want) {
        return None;
    }
    let head = redact(&format!("sha256 of {}", spec["path"].as_str().unwrap_or("the file").trim()));
    Some(if got == want { format!("{head} is {got}, as expected") } else { format!("{head} changed\nexpected {want}\nactual   {got}") })
}

/// Plain words for a predicate, as the person reads it before approving ("responds in under
/// 500 ms"). They say what the stored metric, operator and target mean.
pub fn says(kind: &str, spec: &Value, pred: Option<&Predicate>) -> String {
    let s = |k: &str| spec[k].as_str().unwrap_or("").trim().to_string();
    let rel = |op: &str| match op {
        "eq" => "equal to",
        "ne" => "not equal to",
        "lt" => "below",
        "lte" => "at most",
        "gt" => "above",
        "gte" => "at least",
        _ => "containing",
    };
    let Some(p) = pred else {
        return match kind {
            "file_exists" => format!("{} exists", s("path")),
            "file_contains" => format!("{} contains \"{}\"", s("path"), spec["text"].as_str().unwrap_or("")),
            "file_sha256" => format!("{} is unchanged (sha256 {})", s("path"), s("sha256").chars().take(12).collect::<String>()),
            other => format!("a {other} check"),
        };
    };
    let op = p.operator.as_deref().unwrap_or("");
    let t = &p.target;
    match p.metric.as_str() {
        "status" if op == "ne" => format!("{} doesn't answer with status {t}", s("url")),
        "status" => format!("{} answers with status {t}", s("url")),
        "latency_ms" if op == "lte" => format!("{} responds in {t} ms or less", s("url")),
        "latency_ms" => format!("{} responds in under {t} ms", s("url")),
        "body_contains" => format!("the page at {} contains \"{t}\"", s("url")),
        "json_value" => {
            let at = match s("pointer") {
                p if p.is_empty() => "the JSON".to_string(),
                p => p,
            };
            format!("{at} at {} is {} {t}", s("url"), rel(op))
        }
        "exit_code" if op == "ne" => format!("`{}` doesn't exit with code {t}", s("command")),
        "exit_code" if t == "0" => format!("`{}` succeeds (exit code 0)", s("command")),
        "exit_code" => format!("`{}` exits with code {t}", s("command")),
        "stdout_contains" => format!("`{}` prints \"{t}\"", s("command")),
        "stdout_number" => format!("the first number `{}` prints is {} {t}", s("command"), rel(op)),
        other => format!("{other} {op} {t}"),
    }
}

/// A fix task's check that carries its goal's predicate (`spec.predicate`) is judged by it, so the
/// fix is verified by what the goal means. Returns `(status, output)` as [`run_spec`] does, the
/// predicate's unknown being an error; None without a predicate.
///
/// [`run_spec`]: crate::server::missions::checks::run_spec
pub async fn run_for_check(kind: &str, spec: &Value, folder: Option<&Path>, person_ok: bool) -> Option<(String, String)> {
    let pred = Predicate::from_json(&spec["predicate"])?;
    let out = run(kind, spec, Some(&pred), folder, person_ok).await;
    let status = match out.verdict {
        MET => "passed",
        NOT_MET => "failed",
        _ => "error",
    };
    let wanted = says(kind, spec, Some(&pred));
    let seen = out.observed.as_deref().map(|o| format!(" (observed: {o})")).unwrap_or_default();
    Some((status.into(), format!("{} {wanted}{seen}\n{}", if status == "passed" { "met:" } else { "not met:" }, out.excerpt)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(metric: &str, op: Option<&str>, target: &str) -> Predicate {
        Predicate::new(metric, op, target)
    }

    fn http(code: u16, body: &str) -> Seen {
        Seen { http_status: Some(code), body: Some(body.into()), ..Default::default() }
    }

    fn cmd(code: i64, out: &str) -> Seen {
        Seen { exit_code: Some(code), stdout: Some(out.into()), ..Default::default() }
    }

    fn j(pred: &Predicate, spec: Value, seen: &Seen) -> (&'static str, Option<String>) {
        judge(Some(pred), &spec, "failed", seen, 0)
    }

    #[test]
    fn the_first_number_is_read() {
        assert_eq!(first_number("Total: 42"), Some(42.0));
        assert_eq!(first_number("1,234"), Some(1234.0));
        assert_eq!(first_number("1,234,567 rows"), Some(1_234_567.0));
        assert_eq!(first_number("12,34"), Some(12.0));
        assert_eq!(first_number("3.5"), Some(3.5));
        assert_eq!(first_number("-2"), Some(-2.0));
        assert_eq!(first_number("1e3"), Some(1000.0));
        assert_eq!(first_number("2.5E-1"), Some(0.25));
        assert_eq!(first_number("7 then 9"), Some(7.0));
        assert_eq!(first_number("42abc"), Some(42.0));
        assert_eq!(first_number(".5"), Some(0.5));
        assert_eq!(first_number("a - 2"), Some(2.0));
        assert_eq!(first_number("none here"), None);
        assert_eq!(first_number(""), None);
    }

    #[test]
    fn http_status_and_latency() {
        let ok = p("status", Some("eq"), "200");
        assert_eq!(j(&ok, json!({}), &http(200, "")), (MET, Some("200".into())));
        assert_eq!(j(&ok, json!({}), &http(204, "")).0, NOT_MET);
        assert_eq!(j(&ok, json!({}), &http(500, "")).0, NOT_MET);
        // Redirects aren't followed: a 301 is what was seen
        assert_eq!(j(&ok, json!({}), &http(301, "")), (NOT_MET, Some("301".into())));
        assert_eq!(j(&p("status", Some("ne"), "500"), json!({}), &http(503, "")).0, MET);
        // lt fails at the limit, lte passes
        let lt = p("latency_ms", Some("lt"), "500");
        let lte = p("latency_ms", Some("lte"), "500");
        let at = |pred: &Predicate, ms| judge(Some(pred), &json!({}), "passed", &http(200, ""), ms).0;
        assert_eq!((at(&lt, 499), at(&lt, 500), at(&lt, 501)), (MET, NOT_MET, NOT_MET));
        assert_eq!((at(&lte, 499), at(&lte, 500), at(&lte, 501)), (MET, MET, NOT_MET));
    }

    #[test]
    fn contains_is_literal_and_case_sensitive() {
        let has = p("body_contains", None, "ok");
        assert_eq!(j(&has, json!({}), &http(200, "all ok")), (MET, Some("found".into())));
        assert_eq!(j(&has, json!({}), &http(200, "fine")), (NOT_MET, Some("not found".into())));
        assert_eq!(j(&has, json!({}), &http(200, "OK")).0, NOT_MET);
        let big = format!("{}ok", "x".repeat(5 * 1024 * 1024 - 2));
        assert_eq!(j(&has, json!({}), &http(200, &big)).0, MET);
        let re = p("body_contains", None, "a.*b");
        assert_eq!(j(&re, json!({}), &http(200, "axxb")).0, NOT_MET);
        assert_eq!(j(&re, json!({}), &http(200, "a.*b")).0, MET);
        let sh = p("stdout_contains", None, "$(whoami)");
        assert_eq!(j(&sh, json!({}), &cmd(0, "me")).0, NOT_MET);
        assert_eq!(j(&sh, json!({}), &cmd(0, "literal $(whoami)")).0, MET);
    }

    #[test]
    fn json_values_by_pointer() {
        let body = r#"{"status":"ok","code":200,"text":"200","a":[{"b":3}],"a/b":true,"n":null,"tags":["x","y",5]}"#;
        let at = |ptr: &str, op: &str, t: &str| j(&p("json_value", Some(op), t), json!({ "pointer": ptr }), &http(200, body));
        assert_eq!(at("/status", "eq", "ok"), (MET, Some("ok".into())));
        assert_eq!(at("/a/0/b", "gte", "3"), (MET, Some("3".into())));
        assert_eq!(at("/a~1b", "eq", "true").0, MET);
        assert_eq!(at("/missing", "eq", "x"), (NOT_MET, Some("missing".into())));
        // Lenient: 200 equals "200" either way round
        assert_eq!(at("/code", "eq", "200").0, MET);
        assert_eq!(at("/text", "eq", "200").0, MET);
        assert_eq!(at("/text", "lt", "300").0, MET);
        assert_eq!(at("/n", "eq", "null"), (MET, Some("null".into())));
        assert_eq!(at("/tags", "contains", "y").0, MET);
        assert_eq!(at("/tags", "contains", "5").0, MET);
        assert_eq!(at("/tags", "contains", "z").0, NOT_MET);
        assert_eq!(at("/status", "contains", "o").0, MET);
        // A number can't be ordered against text, nor searched
        assert_eq!(at("/status", "gt", "1").0, UNKNOWN);
        assert_eq!(at("/code", "contains", "2").0, UNKNOWN);
        assert_eq!(at("", "contains", "x").0, UNKNOWN);
        let not_json = j(&p("json_value", Some("eq"), "1"), json!({ "pointer": "/a" }), &http(200, "<html>"));
        assert_eq!(not_json, (UNKNOWN, Some("not JSON".into())));
    }

    #[test]
    fn commands_by_exit_code_and_output() {
        let zero = p("exit_code", Some("eq"), "0");
        assert_eq!(j(&zero, json!({}), &cmd(0, "")), (MET, Some("0".into())));
        assert_eq!(j(&zero, json!({}), &cmd(1, "")).0, NOT_MET);
        assert_eq!(j(&p("exit_code", Some("ne"), "0"), json!({}), &cmd(1, "")).0, MET);
        assert_eq!(j(&p("exit_code", Some("eq"), "1"), json!({}), &cmd(1, "")).0, MET);
        // Not found, not executable, killed: unknown whatever the operator says
        assert_eq!(j(&zero, json!({}), &cmd(127, "")).0, UNKNOWN);
        assert_eq!(j(&p("exit_code", Some("ne"), "0"), json!({}), &cmd(126, "")).0, UNKNOWN);
        assert_eq!(j(&zero, json!({}), &cmd(-1, "")).0, UNKNOWN);
        let n = |op: &str, t: &str, out: &str| j(&p("stdout_number", Some(op), t), json!({}), &cmd(0, out));
        assert_eq!(n("eq", "42", "Total: 42"), (MET, Some("42".into())));
        assert_eq!(n("gt", "1000", "1,234").0, MET);
        assert_eq!(n("lt", "4", "3.5").0, MET);
        assert_eq!(n("lte", "-2", "-2").0, MET);
        assert_eq!(n("gte", "1000", "1e3").0, MET);
        assert_eq!(n("ne", "7", "7 and 9").0, NOT_MET);
        assert_eq!(n("eq", "42", "42abc").0, MET);
        assert_eq!(n("eq", "1", "nothing"), (UNKNOWN, Some("no number".into())));
        assert_eq!(n("eq", "1", "").0, UNKNOWN);
        // The exit code doesn't matter to an output metric
        assert_eq!(j(&p("stdout_contains", None, "ready"), json!({}), &cmd(3, "ready")).0, MET);
    }

    #[test]
    fn errors_are_unknown_and_files_keep_their_meaning() {
        let ok = p("status", Some("eq"), "200");
        assert_eq!(judge(Some(&ok), &json!({}), "error", &Seen::default(), 0), (UNKNOWN, None));
        assert_eq!(judge(None, &json!({}), "passed", &Seen::default(), 0), (MET, None));
        assert_eq!(judge(None, &json!({}), "failed", &Seen::default(), 0), (NOT_MET, None));
        assert_eq!(judge(None, &json!({}), "error", &Seen::default(), 0), (UNKNOWN, None));
    }

    #[test]
    fn predicates_are_checked() {
        let n = |kind: &str, spec: Value, m: Option<&str>, o: Option<&str>, t: Option<&str>| normalize(kind, &spec, m, o, t);
        assert_eq!(n("http", json!({}), None, None, None), Ok(Some(p("status", Some("eq"), "200"))));
        assert_eq!(n("command", json!({}), None, None, None), Ok(Some(p("exit_code", Some("eq"), "0"))));
        assert_eq!(n("file_exists", json!({}), None, None, None), Ok(None));
        assert_eq!(n("http", json!({}), Some("latency_ms"), None, Some("500")), Ok(Some(p("latency_ms", Some("lt"), "500"))));
        assert_eq!(n("http", json!({}), Some("body_contains"), Some("contains"), Some("ok")), Ok(Some(p("body_contains", None, "ok"))));
        assert_eq!(n("command", json!({}), Some("stdout_number"), Some("gte"), Some("1e3")), Ok(Some(p("stdout_number", Some("gte"), "1000"))));
        assert_eq!(n("command", json!({}), Some("stdout_number"), Some("lt"), Some("2.5")), Ok(Some(p("stdout_number", Some("lt"), "2.5"))));
        assert!(n("http", json!({}), Some("latency_ms"), Some("eq"), Some("500")).is_err());
        assert!(n("http", json!({}), Some("latency_ms"), Some("lt"), Some("fast")).is_err());
        assert!(n("http", json!({}), Some("latency_ms"), Some("lt"), Some("-1")).is_err());
        // M1: nothing is under 0 ms; 1 is the lowest target that can be met
        assert!(n("http", json!({}), Some("latency_ms"), Some("lt"), Some("0")).unwrap_err().contains("never be met"));
        assert!(n("http", json!({}), Some("latency_ms"), None, Some("0.0")).is_err());
        assert_eq!(n("http", json!({}), Some("latency_ms"), Some("lt"), Some("1")), Ok(Some(p("latency_ms", Some("lt"), "1"))));
        assert_eq!(n("http", json!({}), Some("latency_ms"), Some("lte"), Some("0")), Ok(Some(p("latency_ms", Some("lte"), "0"))));
        assert!(n("command", json!({}), Some("stdout_number"), Some("lt"), Some("NaN")).is_err());
        assert!(n("command", json!({}), Some("stdout_number"), Some("lt"), Some("Infinity")).is_err());
        assert!(n("command", json!({}), Some("stdout_number"), Some("lt"), Some("1e308")).is_err());
        assert!(n("http", json!({}), Some("body_contains"), None, Some(&"x".repeat(10_000))).is_err());
        assert!(n("http", json!({ "pointer": "a/b" }), Some("json_value"), Some("eq"), Some("1")).is_err());
        assert!(n("http", json!({ "pointer": "/a~2" }), Some("json_value"), Some("eq"), Some("1")).is_err());
        assert!(n("http", json!({ "pointer": "/a~1b" }), Some("json_value"), Some("eq"), Some("1")).is_ok());
        assert!(n("http", json!({}), Some("json_value"), Some("gt"), Some("big")).is_err());
        assert!(n("http", json!({}), Some("speed"), None, Some("1")).is_err());
        assert!(n("http", json!({}), Some("stdout_number"), Some("lt"), Some("1")).is_err());
        assert!(n("http", json!({}), Some("status"), Some("eq"), None).is_err());
        assert!(n("http", json!({}), Some("status"), Some("eq"), Some("700")).is_err());
        assert!(n("command", json!({}), Some("exit_code"), Some("eq"), Some("256")).is_err());
        assert!(n("http", json!({}), None, Some("eq"), Some("200")).is_err());
        assert!(n("file_exists", json!({}), Some("exit_code"), None, None).is_err());
        assert!(n("command", json!({}), Some("stdout_contains"), None, Some("")).is_err());
    }

    #[test]
    fn plain_words_match_the_predicate() {
        let url = json!({ "url": "https://x.test/health" });
        assert_eq!(says("http", &url, Some(&p("latency_ms", Some("lt"), "500"))), "https://x.test/health responds in under 500 ms");
        assert_eq!(says("http", &url, Some(&p("latency_ms", Some("lte"), "500"))), "https://x.test/health responds in 500 ms or less");
        assert_eq!(says("http", &url, Some(&p("status", Some("eq"), "200"))), "https://x.test/health answers with status 200");
        assert_eq!(says("command", &json!({ "command": "make test" }), Some(&p("exit_code", Some("eq"), "0"))), "`make test` succeeds (exit code 0)");
        assert_eq!(says("command", &json!({ "command": "wc -l" }), Some(&p("stdout_number", Some("lte"), "10"))), "the first number `wc -l` prints is at most 10");
        assert_eq!(says("file_exists", &json!({ "path": "out.txt" }), None), "out.txt exists");
    }

    // Post-0.6.5 api-M3: a page past the read cap can't say a needle is missing
    #[test]
    fn a_body_past_the_read_cap_is_cant_check_not_not_met() {
        let cut = Seen { body_truncated: true, ..http(200, "start of a very long page") };
        let has = p("body_contains", None, "needle");
        assert_eq!(j(&has, json!({}), &cut), (UNKNOWN, Some(TOO_BIG.into())));
        assert_eq!(TOO_BIG, "The page is larger than 5 MB, so I can't check it");
        // Found in the part that was read: that is still true
        assert_eq!(j(&p("body_contains", None, "start"), json!({}), &cut).0, MET);
        // A partial document isn't JSON; the reason is the size, not "not JSON"
        let at = json!({ "pointer": "/ok" });
        assert_eq!(j(&p("json_value", Some("eq"), "true"), at, &cut), (UNKNOWN, Some(TOO_BIG.into())));
        // Status and latency don't read the body
        assert_eq!(j(&p("status", Some("eq"), "200"), json!({}), &cut).0, MET);
        // Not past the cap: a missing needle is not met, as before
        assert_eq!(j(&has, json!({}), &http(200, "short")).0, NOT_MET);
    }

    // api-M4: a redirect is not followed; the reason names where it points
    #[test]
    fn a_redirect_says_where_it_points() {
        let moved = Seen { redirect: Some(Some("www.example.com".into())), ..http(301, "") };
        let (v, seen) = j(&p("status", Some("eq"), "200"), json!({}), &moved);
        assert_eq!(v, NOT_MET);
        let seen = seen.unwrap();
        assert!(seen.starts_with("301: it redirects to www.example.com"), "{seen}");
        assert!(seen.contains("use the final address"), "{seen}");
        let same = Seen { redirect: Some(None), ..http(302, "") };
        let seen = j(&p("status", Some("eq"), "200"), json!({}), &same).1.unwrap();
        assert!(seen.contains("another address on the same site"), "{seen}");
        // A body check on the redirect page is not met with the same reason
        assert!(j(&p("body_contains", None, "ok"), json!({}), &moved).1.unwrap().contains("www.example.com"));
        // Met stays plain, and no redirect leaves the value alone
        assert_eq!(j(&p("status", Some("ne"), "200"), json!({}), &moved), (MET, Some("301".into())));
        assert_eq!(j(&p("status", Some("eq"), "200"), json!({}), &http(500, "")), (NOT_MET, Some("500".into())));
    }

    // api-M5: both hashes show, though the excerpt is redacted
    #[tokio::test]
    async fn a_sha256_mismatch_shows_both_hashes() {
        let dir = std::env::temp_dir().join(format!("kwnt-sha-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("release.bin"), "hello").unwrap();
        let actual = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let expected = "a".repeat(64);
        let spec = json!({ "path": "release.bin", "sha256": expected });
        let out = run("file_sha256", &spec, None, Some(dir.as_path()), true).await;
        assert_eq!(out.verdict, NOT_MET);
        assert!(out.excerpt.contains(&format!("expected {expected}")), "{}", out.excerpt);
        assert!(out.excerpt.contains(&format!("actual   {actual}")), "{}", out.excerpt);
        assert!(!out.excerpt.contains("••"), "{}", out.excerpt);
        // A match says so
        let ok = run("file_sha256", &json!({ "path": "release.bin", "sha256": actual.to_uppercase() }), None, Some(dir.as_path()), true).await;
        assert_eq!(ok.verdict, MET);
        assert!(ok.excerpt.contains(actual), "{}", ok.excerpt);
        // Not a hash: nothing is shown unredacted
        let odd = run("file_sha256", &json!({ "path": "release.bin", "sha256": "sk-live-not-a-hash-but-a-secret" }), None, Some(dir.as_path()), true).await;
        assert!(!odd.excerpt.contains("sk-live-not-a-hash-but-a-secret"));
        // A missing file has no hash to show
        let gone = run("file_sha256", &json!({ "path": "gone.bin", "sha256": expected }), None, Some(dir.as_path()), true).await;
        assert!(gone.excerpt.contains("doesn't exist"), "{}", gone.excerpt);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
