//! Mission Control Phase D1: the Performance Market, report-only. A settled task is observed in
//! the background (`record_soon`); `GET /api/performance` groups the observations by task class ×
//! bot (× model) with a Wilson interval on the verified rate. Nothing routes on it: Laya stays the
//! router. See docs/mission-control.md §17.

use super::routes::{bad, internal};
use crate::db::performance::Observation;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Groups with fewer observations than this are `low_confidence`.
pub const MIN_N: usize = 5;
const DEFAULT_DAYS: i64 = 30;

/// Observes `task_id` in the background once it settled. Never blocks or fails the move that
/// settled it: a failure is logged and dropped.
pub fn record_soon(state: &AppState, task_id: &str) {
    let (db, id) = (state.db.clone(), task_id.to_string());
    tokio::spawn(async move {
        if let Err(e) = db.record_performance(&id).await {
            tracing::warn!(task_id = %id, error = %e, "performance observation not recorded");
        }
    });
}

/// The Wilson score interval (z = 1.96) of `k` successes in `n`; `None` when `n` is 0.
pub fn wilson(k: usize, n: usize) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let (z, n, p) = (1.96_f64, n as f64, k as f64 / n as f64);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denom;
    Some(((centre - half).max(0.0), (centre + half).min(1.0)))
}

/// The middle value, or the mean of the two middle ones; `None` when empty.
pub fn median(mut xs: Vec<i64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_unstable();
    let m = xs.len() / 2;
    Some(if xs.len() % 2 == 1 { xs[m] as f64 } else { (xs[m - 1] as f64 + xs[m] as f64) / 2.0 })
}

/// `since` as an RFC 3339 time, a `YYYY-MM-DD` date (midnight UTC) or `Nd` (1–365 days back).
pub fn parse_since(s: Option<&str>, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let Some(s) = s.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(now - Duration::days(DEFAULT_DAYS));
    };
    if let Some(days) = s.strip_suffix('d') {
        return match days.parse::<i64>() {
            Ok(d) if (1..=365).contains(&d) => Ok(now - Duration::days(d)),
            _ => Err("since: use a number of days from 1d to 365d".into()),
        };
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).map(|t| t.and_utc()).unwrap_or(now));
    }
    Err("since: use a time (RFC 3339), a date (YYYY-MM-DD) or a number of days such as 30d".into())
}

fn round(x: f64, places: i32) -> f64 {
    let f = 10_f64.powi(places);
    (x * f).round() / f
}

/// The report's groups, sorted by task class and then by size (largest first).
pub fn group(rows: &[Observation], by_model: bool, names: &HashMap<String, String>) -> Vec<Value> {
    let mut groups: BTreeMap<(String, String, String), Vec<&Observation>> = BTreeMap::new();
    for o in rows {
        let model = if by_model { o.model.clone() } else { String::new() };
        groups.entry((o.task_class.clone(), o.kwant_id.clone(), model)).or_default().push(o);
    }
    let mut out: Vec<(String, usize, Value)> = groups
        .into_iter()
        .map(|((class, kwant_id, model), os)| {
            let n = os.len();
            let count = |r: &str| os.iter().filter(|o| o.result == r).count();
            let verified = count("verified");
            let (lo, hi) = wilson(verified, n).unwrap_or((0.0, 1.0));
            let costs: Vec<f64> = os.iter().filter_map(|o| o.cost_usd).collect();
            let interventions: i64 = os.iter().map(|o| o.interventions).sum();
            // Engines in the group, most common first (a bot's engine can change over time)
            let mut engines: Vec<(String, usize)> = os.iter().fold(HashMap::<String, usize>::new(), |mut m, o| {
                *m.entry(o.engine.clone()).or_default() += 1;
                m
            })
            .into_iter()
            .collect();
            engines.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let mut g = json!({
                "task_class": class,
                "kwant_id": kwant_id,
                "kwant_name": names.get(&kwant_id),
                "engine": engines.first().map(|e| e.0.clone()),
                "n": n,
                "verified": verified,
                "failed": count("failed"),
                "unverified": count("unverified"),
                "verified_rate": round(verified as f64 / n as f64, 4),
                "verified_ci95": [round(lo, 4), round(hi, 4)],
                "median_latency_ms": median(os.iter().map(|o| o.latency_ms).collect()),
                "median_fresh_tokens": median(os.iter().map(|o| o.fresh_tokens).collect()),
                "cost_usd": (!costs.is_empty()).then(|| round(costs.iter().sum::<f64>(), 6)),
                "cost_per_task_usd": (!costs.is_empty()).then(|| round(costs.iter().sum::<f64>() / costs.len() as f64, 6)),
                "interventions_per_task": round(interventions as f64 / n as f64, 4),
                "low_confidence": n < MIN_N,
            });
            if by_model {
                g["model"] = json!(model);
            }
            (class, n, g)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    out.into_iter().map(|(_, _, g)| g).collect()
}

#[derive(Deserialize)]
pub struct ReportQuery {
    class: Option<String>,
    bot: Option<String>,
    since: Option<String>,
    by: Option<String>,
}

/// `GET /api/performance`: person-only (`auth.rs`), read-only.
pub async fn report(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ReportQuery>) -> Response {
    let since = match parse_since(q.since.as_deref(), Utc::now()) {
        Ok(t) => t,
        Err(e) => return bad(e),
    };
    let by_model = match q.by.as_deref().unwrap_or("model") {
        "model" => true,
        "bot" => false,
        _ => return bad("by: use model or bot"),
    };
    let nonblank = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (class, bot) = (nonblank(&q.class), nonblank(&q.bot));
    let rows = match state.db.observations(space.as_str(), since, class.as_deref(), bot.as_deref()).await {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let mut names = HashMap::new();
    for id in rows.iter().map(|o| o.kwant_id.clone()).collect::<std::collections::HashSet<_>>() {
        if let Ok(Some(k)) = state.db.get_kwant(&id).await {
            names.insert(id, k.name);
        }
    }
    Json(json!({
        "since": since,
        "report_only": true,
        "min_n": MIN_N,
        "observations": rows.len(),
        "groups": group(&rows, by_model, &names),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 5e-4
    }

    #[test]
    fn wilson_matches_known_intervals() {
        assert_eq!(wilson(0, 0), None);
        // Reference values (Wilson score, z = 1.96)
        let (lo, hi) = wilson(9, 12).unwrap();
        assert!(close(lo, 0.4677) && close(hi, 0.9111), "{lo} {hi}");
        let (lo, hi) = wilson(0, 10).unwrap();
        assert!(close(lo, 0.0) && close(hi, 0.2775), "{lo} {hi}");
        let (lo, hi) = wilson(10, 10).unwrap();
        assert!(close(lo, 0.7225) && close(hi, 1.0), "{lo} {hi}");
        let (lo, hi) = wilson(1, 2).unwrap();
        assert!(close(lo, 0.0945) && close(hi, 0.9055), "{lo} {hi}");
    }

    #[test]
    fn median_of_odd_and_even_lists() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![7]), Some(7.0));
        assert_eq!(median(vec![30, 10, 20]), Some(20.0));
        assert_eq!(median(vec![40, 10, 30, 20]), Some(25.0));
    }

    #[test]
    fn since_takes_days_dates_and_times() {
        let now = DateTime::parse_from_rfc3339("2026-10-29T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(parse_since(None, now).unwrap(), now - Duration::days(30));
        assert_eq!(parse_since(Some("7d"), now).unwrap(), now - Duration::days(7));
        assert_eq!(parse_since(Some("2026-10-01"), now).unwrap().to_rfc3339(), "2026-10-01T00:00:00+00:00");
        assert_eq!(parse_since(Some("2026-10-02T03:04:05Z"), now).unwrap().to_rfc3339(), "2026-10-02T03:04:05+00:00");
        for bad in ["0d", "366d", "xd", "yesterday", "2026-13-01"] {
            assert!(parse_since(Some(bad), now).is_err(), "{bad}");
        }
    }
}
