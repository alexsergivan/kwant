//! How much of an AI account's 5-hour and weekly limit is used, and when each resets: read from
//! the `rate_limit_event` line Claude's `stream-json` prints on every turn, or from Codex's
//! `account/rateLimits/read`. Kept per profile (`llm_profile_state.usage_json`) and shown as
//! `usage` on `GET /api/profiles` (contract: autonomy/account-usage/brief.md).

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The least time between two "Check now" reads of one account: each Claude read spends a
/// little of the person's limit.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// One limit window: how much of it is used (0–100, rounded) and when it resets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// `five_hour` or `weekly`.
    pub kind: String,
    pub used_pct: u8,
    pub resets_at: Option<DateTime<Utc>>,
}

/// What one turn or check read: the account's state and its windows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReading {
    /// `ok`, `warning` or `out`; `None` when the CLI didn't say.
    pub status: Option<String>,
    /// `five_hour` first, then `weekly`; windows of any other length are dropped.
    pub windows: Vec<UsageWindow>,
}

/// An account's last reading, as `GET /api/profiles` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub checked_at: DateTime<Utc>,
    /// `turn` (read from a bot's turn) or `check` (the person pressed Check now).
    pub source: String,
    pub status: Option<String>,
    pub windows: Vec<UsageWindow>,
}

impl AccountUsage {
    pub fn new(reading: UsageReading, source: &str, checked_at: DateTime<Utc>) -> Self {
        Self { checked_at, source: source.into(), status: reading.status, windows: reading.windows }
    }

    /// The stored form (`usage_json`): everything but `checked_at`, which has its own column.
    pub fn to_stored(&self) -> String {
        serde_json::json!({ "source": self.source, "status": self.status, "windows": self.windows }).to_string()
    }

    /// Reads a stored row back; `None` for a row this version can't read.
    pub fn from_stored(json: &str, checked_at: DateTime<Utc>) -> Option<Self> {
        #[derive(Deserialize)]
        struct Stored {
            source: String,
            status: Option<String>,
            windows: Vec<UsageWindow>,
        }
        let s: Stored = serde_json::from_str(json).ok()?;
        Some(Self { checked_at, source: s.source, status: s.status, windows: s.windows })
    }
}

/// The contract's `status` for Claude's: `allowed` → ok, `allowed_warning` → warning,
/// `rejected` → out; anything else → `None`.
pub fn claude_status(raw: &str) -> Option<String> {
    match raw {
        "allowed" => Some("ok".into()),
        "allowed_warning" => Some("warning".into()),
        "rejected" => Some("out".into()),
        _ => None,
    }
}

/// A fraction (0..1) or percent already (Codex) as 0–100, rounded and clamped.
fn pct(v: f64) -> u8 {
    if v.is_finite() {
        v.round().clamp(0.0, 100.0) as u8
    } else {
        0
    }
}

fn epoch(v: &Value) -> Option<DateTime<Utc>> {
    let secs = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
    // Some builds report milliseconds
    let secs = if secs > 100_000_000_000 { secs / 1000 } else { secs };
    Utc.timestamp_opt(secs, 0).single()
}

fn sorted(mut windows: Vec<UsageWindow>) -> Vec<UsageWindow> {
    windows.sort_by_key(|w| (w.kind != "five_hour") as u8);
    windows.dedup_by(|a, b| a.kind == b.kind);
    windows
}

/// Claude's `{"type":"rate_limit_event","rate_limit_info":{…}}` line, read. `None` when it holds
/// neither a status nor a window.
pub fn from_claude_event(line: &Value) -> Option<UsageReading> {
    let info = &line["rate_limit_info"];
    let status = info["status"].as_str().and_then(claude_status);
    let mut windows = Vec::new();
    for (key, kind) in [("five_hour", "five_hour"), ("seven_day", "weekly")] {
        let w = &info["unifiedWindows"][key];
        let Some(used) = w["utilization"].as_f64() else {
            continue;
        };
        windows.push(UsageWindow { kind: kind.into(), used_pct: pct(used * 100.0), resets_at: epoch(&w["resetsAt"]) });
    }
    (status.is_some() || !windows.is_empty()).then(|| UsageReading { status, windows: sorted(windows) })
}

/// The contract's window kind for a Codex window `minutes` long: 300 → five_hour, 10080 →
/// weekly, anything else dropped.
pub fn codex_kind(minutes: i64) -> Option<&'static str> {
    match minutes {
        300 => Some("five_hour"),
        10080 => Some("weekly"),
        _ => None,
    }
}

/// The `result` of Codex's `account/rateLimits/read` (`{"rateLimits":{"primary":{"usedPercent",
/// "windowDurationMins","resetsAt"},"secondary":…,"rateLimitReachedType":…},…}`), read. `None`
/// when it has no window of a known length.
pub fn from_codex_rate_limits(result: &Value) -> Option<UsageReading> {
    let limits = &result["rateLimits"];
    let windows: Vec<UsageWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| {
            let w = &limits[*k];
            let kind = codex_kind(w["windowDurationMins"].as_i64()?)?;
            let used = w["usedPercent"].as_f64()?;
            Some(UsageWindow { kind: kind.into(), used_pct: pct(used), resets_at: epoch(&w["resetsAt"]) })
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    let reached = !limits["rateLimitReachedType"].is_null() || result["ordinaryUsageAllowed"] == false;
    let status = if reached || windows.iter().any(|w| w.used_pct >= 100) { "out" } else { "ok" };
    Some(UsageReading { status: Some(status.into()), windows: sorted(windows) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines recorded from claude 2.1.323 `-p --output-format stream-json --verbose` (ids blanked).
    const RECORDED: &str = include_str!("../../tests/fixtures/claude_rate_limit_event.jsonl");

    fn recorded() -> Value {
        RECORDED.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|v| v["type"] == "rate_limit_event").unwrap()
    }

    #[test]
    fn a_recorded_rate_limit_event_reads_as_two_windows() {
        let r = from_claude_event(&recorded()).unwrap();
        assert_eq!(r.status.as_deref(), Some("ok"));
        assert_eq!(
            r.windows,
            vec![
                UsageWindow { kind: "five_hour".into(), used_pct: 14, resets_at: Utc.timestamp_opt(1794605400, 0).single() },
                UsageWindow { kind: "weekly".into(), used_pct: 56, resets_at: Utc.timestamp_opt(1794960000, 0).single() },
            ]
        );
    }

    #[test]
    fn claude_statuses_map_to_the_contract() {
        assert_eq!(claude_status("allowed").as_deref(), Some("ok"));
        assert_eq!(claude_status("allowed_warning").as_deref(), Some("warning"));
        assert_eq!(claude_status("rejected").as_deref(), Some("out"));
        assert_eq!(claude_status("something_new"), None);
        let line = serde_json::json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","unifiedWindows":{"seven_day":{"utilization":0.826,"resetsAt":1795136340}}}});
        let r = from_claude_event(&line).unwrap();
        assert_eq!(r.status.as_deref(), Some("warning"));
        assert_eq!(r.windows.len(), 1);
        assert_eq!((r.windows[0].kind.as_str(), r.windows[0].used_pct), ("weekly", 83));
    }

    #[test]
    fn an_empty_event_is_no_reading() {
        assert!(from_claude_event(&serde_json::json!({"type":"rate_limit_event","rate_limit_info":{}})).is_none());
        assert!(from_claude_event(&serde_json::json!({"type":"rate_limit_event"})).is_none());
    }

    #[test]
    fn codex_windows_map_by_length_and_others_are_dropped() {
        let v = serde_json::json!({"rateLimits":{
            "primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1794608940},
            "secondary":{"usedPercent":40,"windowDurationMins":10080,"resetsAt":1795136340},
            "rateLimitReachedType":null}});
        let r = from_codex_rate_limits(&v).unwrap();
        assert_eq!(r.status.as_deref(), Some("ok"));
        assert_eq!(r.windows.iter().map(|w| (w.kind.as_str(), w.used_pct)).collect::<Vec<_>>(), vec![("five_hour", 12), ("weekly", 40)]);
        // Weekly reported first, an odd window dropped, a reached limit is out
        let v = serde_json::json!({"rateLimits":{
            "primary":{"usedPercent":100,"windowDurationMins":10080,"resetsAt":null},
            "secondary":{"usedPercent":3,"windowDurationMins":60},
            "rateLimitReachedType":"rate_limit_reached"}});
        let r = from_codex_rate_limits(&v).unwrap();
        assert_eq!(r.status.as_deref(), Some("out"));
        assert_eq!(r.windows, vec![UsageWindow { kind: "weekly".into(), used_pct: 100, resets_at: None }]);
        assert!(from_codex_rate_limits(&serde_json::json!({"rateLimits":{"primary":null}})).is_none());
    }

    #[test]
    fn stored_usage_reads_back() {
        let at = Utc.timestamp_opt(1794600000, 0).single().unwrap();
        let u = AccountUsage::new(from_claude_event(&recorded()).unwrap(), "turn", at);
        assert_eq!(AccountUsage::from_stored(&u.to_stored(), at), Some(u.clone()));
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["checked_at"], "2026-11-13T20:00:00Z");
        assert_eq!(v["windows"][0]["resets_at"], "2026-11-13T21:30:00Z");
        assert!(AccountUsage::from_stored("not json", at).is_none());
    }
}
