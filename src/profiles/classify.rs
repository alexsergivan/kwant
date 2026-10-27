//! Sorts a failed provider call into quota, auth or transient, from its status and error text.

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailClass {
    /// Usage limit, out of credits or billing: another account may still work.
    Quota,
    /// Signed out, bad or expired key: another account may still work.
    Auth,
    /// Overloaded, rate limited, timeouts: retry the same account, don't fail over.
    Transient,
}

impl FailClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FailClass::Quota => "quota",
            FailClass::Auth => "auth",
            FailClass::Transient => "transient",
        }
    }

    /// Whether moving to another account of the same engine can help.
    pub fn fails_over(self) -> bool {
        matches!(self, FailClass::Quota | FailClass::Auth)
    }
}

/// A provider call that failed before the model did anything visible.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderFailure {
    pub class: FailClass,
    /// When a quota resets, if the provider said.
    pub retry_at: Option<DateTime<Utc>>,
    /// A short, redacted excerpt of the provider's message, for the chat notice only.
    pub detail: String,
}

/// How long a quota-limited profile is skipped when the provider gave no reset time.
pub const DEFAULT_QUOTA_COOLDOWN_MINS: i64 = 60;

const QUOTA_PATTERNS: &[&str] = &[
    "usage limit",
    "hit your limit",
    "reached your limit",
    "out of credits",
    "insufficient_quota",
    "exceeded your current quota",
    "usage_limit_exceeded",
    "credit balance is too low",
    "billing_error",
    "payment required",
    "quota",
];

const AUTH_PATTERNS: &[&str] = &[
    "not logged in",
    "not signed in",
    "please run /login",
    "please log in",
    "invalid api key",
    "invalid x-api-key",
    "incorrect api key",
    "invalid_api_key",
    "authentication_error",
    "401 unauthorized",
    "missing bearer",
    "oauth token has expired",
    "token has expired",
    "permission_error",
];

const TRANSIENT_PATTERNS: &[&str] = &[
    "overloaded",
    "rate limit",
    "rate_limit",
    "too many requests",
    "timed out",
    "timeout",
    "temporarily unavailable",
    "internal server error",
    "bad gateway",
    "service unavailable",
    "connection reset",
    "connection refused",
];

/// Classifies a failed call. `status` is the HTTP status when there was one. Returns `None` when
/// nothing marks it as a provider failure (then it's an ordinary error and nothing fails over).
pub fn classify(
    status: Option<u16>,
    text: &str,
    now: DateTime<Utc>,
) -> Option<(FailClass, Option<DateTime<Utc>>)> {
    let lower = text.to_lowercase();
    let quota = QUOTA_PATTERNS.iter().any(|p| lower.contains(p)) || status == Some(402);
    if quota {
        return Some((FailClass::Quota, Some(quota_reset(text, &lower, now))));
    }
    if AUTH_PATTERNS.iter().any(|p| lower.contains(p)) || matches!(status, Some(401) | Some(403)) {
        return Some((FailClass::Auth, None));
    }
    let transient_status =
        matches!(status, Some(408) | Some(429) | Some(529)) || status.is_some_and(|s| s >= 500);
    if transient_status || TRANSIENT_PATTERNS.iter().any(|p| lower.contains(p)) {
        return Some((FailClass::Transient, None));
    }
    None
}

/// When a quota resets: Claude's `...limit reached|<epoch>`, "try again in 3 hours 20 minutes",
/// "resets in 45m", else now plus the default cooldown. Never more than 7 days out.
fn quota_reset(text: &str, lower: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let fallback = now + Duration::minutes(DEFAULT_QUOTA_COOLDOWN_MINS);
    let at = epoch_after_pipe(text, now).or_else(|| relative_reset(lower).map(|d| now + d));
    match at {
        Some(t) if t > now && t <= now + Duration::days(7) => t,
        _ => fallback,
    }
}

fn epoch_after_pipe(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (_, rest) = text.split_once('|')?;
    let digits: String = rest
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let secs: i64 = digits.parse().ok()?;
    // Seconds or milliseconds
    let secs = if secs > 100_000_000_000 {
        secs / 1000
    } else {
        secs
    };
    let at = Utc.timestamp_opt(secs, 0).single()?;
    (at > now - Duration::days(1)).then_some(at)
}

/// Sums "<n> day(s)/hour(s)/minute(s)/second(s)" (also "3h 20m", "3h20m") after "in ".
fn relative_reset(lower: &str) -> Option<Duration> {
    let start = [
        "try again in ",
        "resets in ",
        "reset in ",
        "again in ",
        "available in ",
    ]
    .iter()
    .find_map(|k| lower.find(k).map(|i| i + k.len()))?;
    let tail: String = lower[start..].chars().take(60).collect();
    // "3 hours" and "3hours" both become "3hours"; "and" and commas are skipped
    let mut glued: Vec<String> = Vec::new();
    for word in tail.split(|c: char| c.is_whitespace() || c == ',') {
        let word = word.trim_end_matches(['.', ';', ')']);
        if word.is_empty() || word == "and" {
            continue;
        }
        match glued.last_mut() {
            Some(prev)
                if prev.chars().all(|c| c.is_ascii_digit())
                    && word.starts_with(|c: char| c.is_ascii_alphabetic()) =>
            {
                prev.push_str(word)
            }
            _ => glued.push(word.to_string()),
        }
    }
    let mut total = Duration::zero();
    let mut found = false;
    for word in glued {
        let mut rest = word.as_str();
        let mut any = false;
        while !rest.is_empty() {
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits == 0 {
                break;
            }
            let n: i64 = rest[..digits].parse().ok()?;
            rest = &rest[digits..];
            let letters = rest.len()
                - rest
                    .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                    .len();
            let unit = &rest[..letters];
            rest = &rest[letters..];
            total += match unit {
                u if u.starts_with('d') => Duration::days(n),
                u if u.starts_with('h') => Duration::hours(n),
                u if u.starts_with("ms") => break,
                u if u.starts_with('m') => Duration::minutes(n),
                u if u.starts_with('s') => Duration::seconds(n),
                _ => break,
            };
            any = true;
        }
        if !any {
            break;
        }
        found = true;
    }
    found.then_some(total)
}

/// Key names (matched case-insensitively, as the whole name or its suffix, so `client_secret`,
/// `x-api-key` and `AWS_SECRET_ACCESS_KEY` count) whose value is a credential when written as
/// `name=value`, `name: value` or, for a `--flag`, `--name value`.
const CREDENTIAL_KEYS: [&str; 16] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "secret_key",
    "access_key",
    "access_key_id",
    "private_key",
    "private-key",
    "privatekey",
    "authorization",
    "cookie",
];
/// Short key names that only count whole or after `_`/`-` (`db_pass`, `--pass`), so `bypass=1`
/// and `compass: north` are left alone.
const SHORT_CREDENTIAL_KEYS: [&str; 1] = ["pass"];
/// Key names that only count after `_`/`-` (`MYSQL_PWD`, `--pwd`), so the shell's `PWD=/home/x`
/// and `OLDPWD=…` stay readable.
const SEPARATED_CREDENTIAL_KEYS: [&str; 1] = ["pwd"];
/// Slack token prefixes; the whole token is masked (`xoxb-123-abc` → `[redacted]`).
const SLACK_TOKEN_PREFIXES: [&str; 7] = [
    "xoxb-", "xoxp-", "xoxa-", "xoxs-", "xoxe-", "xoxr-", "xapp-",
];

/// An AWS access key ID: `AKIA` (long-term) or `ASIA` (temporary) plus 16 uppercase letters or
/// digits, as a whole word, so "AKIA" alone or a short `AKIA123` stays readable.
fn is_aws_access_key_id(word: &str) -> bool {
    word.len() == 20
        && (word.starts_with("AKIA") || word.starts_with("ASIA"))
        && word[4..]
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn is_credential_key(name: &str) -> bool {
    let after_separator = |k: &&str, whole: bool| {
        name.strip_suffix(*k)
            .is_some_and(|rest| (whole && rest.is_empty()) || rest.ends_with(['_', '-']))
    };
    CREDENTIAL_KEYS.iter().any(|k| name.ends_with(k))
        || SHORT_CREDENTIAL_KEYS
            .iter()
            .any(|k| after_separator(k, true))
        || SEPARATED_CREDENTIAL_KEYS
            .iter()
            .any(|k| after_separator(k, false))
}

/// Masks the password in URL userinfo, keeping the user and host:
/// `postgres://bob:hunter2@db/x` → `postgres://bob:[redacted]@db/x`.
fn mask_url_passwords(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(p) = rest.find("://") {
        let after = &rest[p + 3..];
        out.push_str(&rest[..p + 3]);
        let len = after
            .find(|c: char| c.is_whitespace() || "/?#\"'<>`".contains(c))
            .unwrap_or(after.len());
        let authority = &after[..len];
        // the last `@` ends the userinfo, so a password with a raw `@` is still hidden whole
        if let Some(at) = authority.rfind('@') {
            if let Some(colon) = authority[..at].find(':').filter(|&c| at > c + 1) {
                out.push_str(&authority[..=colon]);
                out.push_str("[redacted]");
                out.push_str(&authority[at..]);
                rest = &after[len..];
                continue;
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Masks free-text credentials, keeping the key: `password=hunter2`, `db_pwd: hunter2`,
/// `"api_key":"…"`, `--password hunter2`, `Authorization: Bearer …` and a bare `Bearer …` become
/// `password=[redacted]`, `Bearer [redacted]` and so on, a `Cookie:`/`Set-Cookie:` value is
/// hidden to the end of the line, a Slack token (`xoxb-…`) or AWS access key ID (`AKIA…`) is
/// hidden whole, and a password in a
/// URL becomes `postgres://bob:[redacted]@db`. Only a key followed by `=` or `:` (or a `--flag`
/// followed by a value) counts, so prose like "the password field is empty" is left alone.
pub fn mask_credentials(text: &str) -> String {
    let text = &mask_url_passwords(text);
    let is_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let skip_spaces = |mut j: usize| {
        while j < text.len() && text.as_bytes()[j] == b' ' {
            j += 1;
        }
        j
    };
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if !is_name(bytes[i]) || (i > 0 && is_name(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_name(bytes[i]) {
            i += 1;
        }
        let name = text[start..i].to_ascii_lowercase();
        if SLACK_TOKEN_PREFIXES
            .iter()
            .any(|p| name.len() > p.len() && name.starts_with(p))
            || is_aws_access_key_id(&text[start..i])
        {
            out.push_str(&text[copied..start]);
            out.push_str("[redacted]");
            copied = i;
            continue;
        }
        let mut j;
        if name == "bearer" {
            // a bare `Bearer <token>`; an all-lowercase word is prose ("Bearer token", "bearer of")
            j = skip_spaces(i);
            let word = &text[j..];
            let word = &word[..word.find(char::is_whitespace).unwrap_or(word.len())];
            if j == i || word.is_empty() || word.bytes().all(|b| b.is_ascii_lowercase()) {
                continue;
            }
        } else {
            if !is_credential_key(&name) {
                continue;
            }
            // name, an optional closing quote, then `=` or `:`; or `--flag value`
            j = i;
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            let after_name = j;
            j = skip_spaces(j);
            if j < bytes.len() && (bytes[j] == b'=' || bytes[j] == b':') {
                j = skip_spaces(j + 1);
            } else {
                let flag = name.starts_with("--") && !name.starts_with("--no-");
                if !(flag && after_name == i && j > i && j < bytes.len() && bytes[j] != b'-') {
                    continue;
                }
            }
            // `Authorization: Bearer abc` keeps its scheme and hides the credential after it
            if name.ends_with("authorization") {
                let scheme = text[j..]
                    .find(|c: char| !c.is_ascii_alphabetic())
                    .map_or(bytes.len(), |e| j + e);
                let value = skip_spaces(scheme);
                if scheme > j && bytes.get(scheme) == Some(&b' ') && value < bytes.len() {
                    j = value;
                } else if value >= bytes.len()
                    && ["bearer", "basic", "digest"]
                        .contains(&text[j..scheme].to_ascii_lowercase().as_str())
                {
                    // a scheme name with no credential after it
                    continue;
                }
            }
        }
        // a quoted value up to its closing quote, otherwise up to whitespace or punctuation
        let (from, to) = match bytes.get(j) {
            Some(&q) if q == b'"' || q == b'\'' => {
                let end = text[j + 1..].find(q as char).map_or(bytes.len(), |e| j + 1 + e);
                (j + 1, end)
            }
            // a cookie header holds several `name=value; …` pairs, so it is hidden to the line end
            _ if name.ends_with("cookie") => {
                let end = text[j..].find(['\n', '\r']).map_or(bytes.len(), |e| j + e);
                (j, text[j..end].trim_end().len() + j)
            }
            // already masked (text that went through here once): leave it, or the `]` would be
            // left behind as `[redacted]]`
            _ if text[j..].starts_with("[redacted]") => (j, j),
            _ => {
                let end = text[j..]
                    .find(|c: char| c.is_whitespace() || ",;&)]}\"'".contains(c))
                    .map_or(bytes.len(), |e| j + e);
                (j, end)
            }
        };
        if to > from {
            out.push_str(&text[copied..from]);
            out.push_str("[redacted]");
            copied = to;
            i = to;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Shortens provider error text for a chat notice: one line, at most `max` chars, with anything
/// that looks like a key or token masked.
pub fn redact_detail(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let line = mask_credentials(line);
    let masked: Vec<String> = line
        .split(' ')
        .map(|w| {
            let looks_secret = w.len() >= 20
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.=/+".contains(c))
                && w.chars().any(|c| c.is_ascii_digit())
                && !w.starts_with("http");
            let prefixed = ["sk-", "xai-", "sess-", "Bearer"]
                .iter()
                .any(|p| w.starts_with(p))
                && w.len() > 8;
            if looks_secret || prefixed {
                "[redacted]".to_string()
            } else {
                w.to_string()
            }
        })
        .collect();
    let joined = masked.join(" ");
    if joined.chars().count() > max {
        let cut: String = joined.chars().take(max).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap()
    }

    #[test]
    fn quota_messages_from_each_cli() {
        for text in [
            "Claude AI usage limit reached|1792162800",
            "You've hit your limit · resets 5pm",
            "You've hit your usage limit. Upgrade to Pro or try again in 3 hours 20 minutes.",
            "Error: insufficient_quota: You exceeded your current quota",
            "Your credit balance is too low to access the Anthropic API",
            "usage_limit_exceeded",
        ] {
            let (class, at) = classify(None, text, now()).unwrap();
            assert_eq!(class, FailClass::Quota, "{text}");
            assert!(at.unwrap() > now(), "{text}");
        }
        assert_eq!(classify(Some(402), "", now()).unwrap().0, FailClass::Quota);
    }

    #[test]
    fn quota_reset_times() {
        let (_, at) = classify(None, "Claude AI usage limit reached|1792162800", now()).unwrap();
        assert_eq!(at.unwrap().timestamp(), 1792162800);
        let (_, at) =
            classify(None, "usage limit. try again in 3 hours 20 minutes", now()).unwrap();
        assert_eq!(at.unwrap(), now() + Duration::minutes(200));
        let (_, at) = classify(None, "usage limit; try again in 2h15m.", now()).unwrap();
        assert_eq!(at.unwrap(), now() + Duration::minutes(135));
        let (_, at) = classify(
            None,
            "quota exceeded, try again in 1 day and 2 hours",
            now(),
        )
        .unwrap();
        assert_eq!(at.unwrap(), now() + Duration::hours(26));
        // No time, a past time, or one absurdly far out: the default cooldown
        let default = now() + Duration::minutes(DEFAULT_QUOTA_COOLDOWN_MINS);
        assert_eq!(
            classify(None, "usage limit reached", now())
                .unwrap()
                .1
                .unwrap(),
            default
        );
        assert_eq!(
            classify(None, "usage limit reached|1000", now())
                .unwrap()
                .1
                .unwrap(),
            default
        );
        assert_eq!(
            classify(None, "usage limit, try again in 90 days", now())
                .unwrap()
                .1
                .unwrap(),
            default
        );
    }

    #[test]
    fn auth_messages_from_each_cli() {
        for text in [
            "Not logged in · Please run /login",
            "Not signed in. Run `grok login`.",
            "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header",
            "invalid x-api-key",
            "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}",
            "OAuth token has expired",
        ] {
            assert_eq!(
                classify(None, text, now()).unwrap().0,
                FailClass::Auth,
                "{text}"
            );
        }
        assert_eq!(classify(Some(401), "", now()).unwrap().0, FailClass::Auth);
        assert_eq!(classify(Some(403), "", now()).unwrap().0, FailClass::Auth);
    }

    #[test]
    fn transient_is_not_failover() {
        for (status, text) in [
            (Some(529), "Overloaded"),
            (
                Some(429),
                "rate_limit_error: Number of requests has exceeded your rate limit",
            ),
            (Some(500), ""),
            (None, "request timed out"),
        ] {
            let (class, at) = classify(status, text, now()).unwrap();
            assert_eq!(class, FailClass::Transient, "{text}");
            assert!(!class.fails_over());
            assert!(at.is_none());
        }
    }

    #[test]
    fn quota_wins_over_auth_and_transient() {
        // A 429 that is about quota, not request rate
        assert_eq!(
            classify(Some(429), "insufficient_quota", now()).unwrap().0,
            FailClass::Quota
        );
    }

    #[test]
    fn ordinary_errors_are_not_classified() {
        assert!(classify(None, "error: file not found: foo.rs", now()).is_none());
        assert!(classify(None, "", now()).is_none());
        assert!(classify(
            Some(400),
            "invalid_request_error: messages: field required",
            now()
        )
        .is_none());
    }

    #[test]
    fn redaction_masks_keys_and_shortens() {
        let text = "invalid x-api-key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 for org\nsecond line";
        let out = redact_detail(text, 200);
        assert!(!out.contains("abcdefghij"), "{out}");
        assert!(!out.contains("second line"));
        assert!(out.contains("[redacted]"));
        assert_eq!(redact_detail(&"x".repeat(300), 10).chars().count(), 11);
    }

    #[test]
    fn free_text_credentials_keep_the_key_and_lose_the_value() {
        for (text, want) in [
            ("login failed password=hunter2 for bob", "login failed password=[redacted] for bob"),
            ("password: hunter2", "password: [redacted]"),
            ("PASSWORD=Hunter2", "PASSWORD=[redacted]"),
            ("MYSQL_PWD=hunter2", "MYSQL_PWD=[redacted]"),
            ("passwd=hunter2&user=bob", "passwd=[redacted]&user=bob"),
            ("secret=s3cr3t", "secret=[redacted]"),
            ("client_secret: abc", "client_secret: [redacted]"),
            ("token=abc.def, then", "token=[redacted], then"),
            ("api_key=abc123", "api_key=[redacted]"),
            ("X-Api-Key: abc123", "X-Api-Key: [redacted]"),
            (
                r#"{"user":"bob","password":"hunter 2","api_key": "abc"}"#,
                r#"{"user":"bob","password":"[redacted]","api_key": "[redacted]"}"#,
            ),
            ("password='hunter2'", "password='[redacted]'"),
        ] {
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn prose_about_credentials_is_left_alone() {
        for text in [
            "the password field is empty",
            "Password incorrect, please try again",
            "your token expired",
            "max_tokens=4096 and tokens: 12",
            "password:",
            "keyboard=us",
        ] {
            assert_eq!(redact_detail(text, 500), text);
        }
    }
    #[test]
    fn url_passwords_are_masked_but_user_and_host_stay() {
        for (text, want) in [
            (
                "connect postgres://bob:hunter2@db:5432/x failed",
                "connect postgres://bob:[redacted]@db:5432/x failed",
            ),
            ("https://user:pass@host/x", "https://user:[redacted]@host/x"),
            (
                "mysql://root:p@ss@10.0.0.1/db",
                "mysql://root:[redacted]@10.0.0.1/db",
            ),
            (
                r#"{"url":"redis://:s3cret@cache:6379"}"#,
                r#"{"url":"redis://:[redacted]@cache:6379"}"#,
            ),
            (
                "a ftp://u:x@h and amqp://u:y@q",
                "a ftp://u:[redacted]@h and amqp://u:[redacted]@q",
            ),
        ] {
            assert_eq!(mask_credentials(text), want, "{text}");
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn authorization_and_bearer_values_are_masked() {
        for (text, want) in [
            (
                "Authorization: Bearer abc123",
                "Authorization: Bearer [redacted]",
            ),
            (
                "authorization: basic Ym9iOmh1bnRlcjI=",
                "authorization: basic [redacted]",
            ),
            (
                "Proxy-Authorization: Basic Ym9i",
                "Proxy-Authorization: Basic [redacted]",
            ),
            ("Authorization: abc123", "Authorization: [redacted]"),
            ("sent Bearer abc123 to api", "sent Bearer [redacted] to api"),
            ("BEARER Xyz", "BEARER [redacted]"),
            (
                r#"{"Authorization":"Bearer abc"}"#,
                r#"{"Authorization":"[redacted]"}"#,
            ),
        ] {
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn space_separated_secret_flags_are_masked() {
        for (text, want) in [
            (
                "run --password hunter2 --user bob",
                "run --password [redacted] --user bob",
            ),
            (
                "--token abc --secret def",
                "--token [redacted] --secret [redacted]",
            ),
            ("cli --api-key abc123", "cli --api-key [redacted]"),
            ("--pass hunter2", "--pass [redacted]"),
            (
                r#"--password "my secret phrase""#,
                r#"--password "[redacted]""#,
            ),
        ] {
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn more_credential_key_names_are_masked() {
        for (text, want) in [
            ("pass=hunter2", "pass=[redacted]"),
            ("db_pass: hunter2", "db_pass: [redacted]"),
            (
                "AWS_SECRET_ACCESS_KEY=abc",
                "AWS_SECRET_ACCESS_KEY=[redacted]",
            ),
            ("DJANGO_SECRET_KEY=abc", "DJANGO_SECRET_KEY=[redacted]"),
            (
                "GITHUB_TOKEN=abc SLACK_SECRET=def",
                "GITHUB_TOKEN=[redacted] SLACK_SECRET=[redacted]",
            ),
            (
                "PGPASSWORD=abc DB_PASSWORD=def",
                "PGPASSWORD=[redacted] DB_PASSWORD=[redacted]",
            ),
            (
                "password=\"my secret phrase\" ok",
                "password=\"[redacted]\" ok",
            ),
        ] {
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn lookalikes_of_new_rules_are_left_alone() {
        for text in [
            "secret santa",
            "passenger count",
            "bypass=1",
            "compass: north",
            "https://host/x",
            "git@github.com:a/b",
            "ssh://git@github.com/a/b",
            "mail bob@host:25",
            "ssh -p 22 host",
            "mkdir -p dir",
            "missing Bearer token",
            "the bearer of bad news",
            "--password-file /etc/pw",
            "--no-password local",
            "--token-limit 4096",
            "--password",
            "Authorization: Bearer",
        ] {
            assert_eq!(redact_detail(text, 500), text, "{text}");
        }
    }

    #[test]
    fn passphrase_private_key_and_cookie_values_are_masked() {
        for (text, want) in [
            ("passphrase=abc", "passphrase=[redacted]"),
            (
                "ssh passphrase: correct horse",
                "ssh passphrase: [redacted] horse",
            ),
            ("GPG_PASSPHRASE=abc", "GPG_PASSPHRASE=[redacted]"),
            ("private_key=abc123", "private_key=[redacted]"),
            ("SSH_PRIVATE_KEY=abc", "SSH_PRIVATE_KEY=[redacted]"),
            (
                r#"{"private_key":"-----BEGIN"}"#,
                r#"{"private_key":"[redacted]"}"#,
            ),
            ("Cookie: session=abc123", "Cookie: [redacted]"),
            ("Cookie: a=1; sid=xyz; theme=dark", "Cookie: [redacted]"),
            (
                "Set-Cookie: sid=abc; Path=/; HttpOnly",
                "Set-Cookie: [redacted]",
            ),
            ("set-cookie:sid=abc", "set-cookie:[redacted]"),
            (
                r#"{"Cookie":"sid=abc; a=b","x":1}"#,
                r#"{"Cookie":"[redacted]","x":1}"#,
            ),
            ("curl --cookie sid=abc", "curl --cookie [redacted]"),
        ] {
            assert_eq!(mask_credentials(text), want, "{text}");
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
        // a cookie value stops at the line end
        assert_eq!(
            mask_credentials("Cookie: sid=abc\nnext line"),
            "Cookie: [redacted]\nnext line"
        );
    }

    #[test]
    fn working_directory_variables_stay_readable() {
        for text in ["PWD=/home/x", "OLDPWD=/tmp", "pwd: /srv/app", "cwd=/home/x"] {
            assert_eq!(mask_credentials(text), text, "{text}");
            assert_eq!(redact_detail(text, 500), text, "{text}");
        }
        for (text, want) in [
            ("password=hunter2", "password=[redacted]"),
            ("passwd=hunter2", "passwd=[redacted]"),
            ("--passwd hunter2", "--passwd [redacted]"),
            ("MYSQL_PWD=hunter2", "MYSQL_PWD=[redacted]"),
            ("db-pwd: hunter2", "db-pwd: [redacted]"),
            ("--pwd hunter2", "--pwd [redacted]"),
        ] {
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn slack_tokens_are_masked_whole() {
        for (text, want) in [
            ("token xoxb-1234-abcd rejected", "token [redacted] rejected"),
            ("xoxp-1-2-3-abcdef", "[redacted]"),
            ("used xoxa-2-abc, xoxs-9-z", "used [redacted], [redacted]"),
            ("XOXB-1234-ABCD", "[redacted]"),
            ("app token xapp-1-A0B1-123-abc", "app token [redacted]"),
            (r#"{"bot":"xoxb-1-abc"}"#, r#"{"bot":"[redacted]"}"#),
        ] {
            assert_eq!(mask_credentials(text), want, "{text}");
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn aws_keys_are_masked() {
        for (text, want) in [
            ("key AKIAIOSFODNN7EXAMPLE denied", "key [redacted] denied"),
            ("AKIAABCDEFGHIJKLMNOP", "[redacted]"),
            ("temp ASIAJEXAMPLEXEKEXAMP, retry", "temp [redacted], retry"),
            (r#"{"id":"AKIAIOSFODNN7EXAMPLE"}"#, r#"{"id":"[redacted]"}"#),
            ("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE", "AWS_ACCESS_KEY_ID=[redacted]"),
            ("aws_access_key_id = abc", "aws_access_key_id = [redacted]"),
            ("access_key_id=AKID", "access_key_id=[redacted]"),
            (
                "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                "AWS_SECRET_ACCESS_KEY=[redacted]",
            ),
            (r#""aws_secret_access_key": "abc""#, r#""aws_secret_access_key": "[redacted]""#),
        ] {
            assert_eq!(mask_credentials(text), want, "{text}");
            assert_eq!(redact_detail(text, 500), want, "{text}");
        }
    }

    #[test]
    fn aws_lookalikes_are_left_alone() {
        for text in [
            "AKIA",
            "Akiane",
            "AKIA123",
            "ASIA pacific",
            "AKIA-1234",
            "ASIAN FOOD",
            "key_id=12",
        ] {
            assert_eq!(mask_credentials(text), text, "{text}");
            assert_eq!(redact_detail(text, 500), text, "{text}");
        }
        // wrong case or length is not a key ID; (`redact_detail` hides these anyway as long
        // words with digits, so only `mask_credentials` is checked)
        for text in ["akiaiosfodnn7example", "AKIAIOSFODNN7EXAMPLEX", "AKIAiosfodnn7example"] {
            assert_eq!(mask_credentials(text), text, "{text}");
        }
        assert_eq!(mask_credentials("AKIAÄÖÜ1234567890 ок"), "AKIAÄÖÜ1234567890 ок");
    }

    #[test]
    fn batch6_lookalikes_are_left_alone() {
        for text in [
            "cookie policy",
            "Cookies accepted",
            "cookies: 3",
            "--cookie-jar /tmp/jar",
            "xoxo",
            "xoxo, gossip girl",
            "xoxb-",
            "skip",
            "sky-high",
            "passphrases are long",
            "private_key_path=/etc/key.pem",
            "private key missing",
            "token expired",
            "max_tokens=4096",
            "bypass=1",
            "compass: north",
            "ssh -p 8080 host",
            "https://user@host",
            "--no-password",
            "passwordless login",
        ] {
            assert_eq!(mask_credentials(text), text, "{text}");
            assert_eq!(redact_detail(text, 500), text, "{text}");
        }
    }

    #[test]
    fn masking_never_panics_on_utf8() {
        let nasty = "🔑 postgres://ü:пароль@хост/ä Authorization: Bearer 秘密 --password 🙈 pass=é token: 鍵 passphrase=ключ xoxb-1-é Cookie: 🍪=秘\nPWD=/дом ";
        let out = mask_credentials(nasty);
        assert!(
            !out.contains("пароль")
                && !out.contains("秘密")
                && !out.contains('🙈')
                && !out.contains("ключ")
                && !out.contains('🍪')
                && out.contains("PWD=/дом"),
            "{out}"
        );
        for (i, _) in nasty.char_indices() {
            let _ = mask_credentials(&nasty[..i]);
            let _ = mask_credentials(&nasty[i..]);
            let _ = redact_detail(&nasty[..i], 40);
        }
    }
}
