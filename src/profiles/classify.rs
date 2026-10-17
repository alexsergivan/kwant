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

/// Shortens provider error text for a chat notice: one line, at most `max` chars, with anything
/// that looks like a key or token masked.
pub fn redact_detail(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
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
}
