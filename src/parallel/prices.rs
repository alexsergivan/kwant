//! The dated price table (`prices.json`, compiled in) and the monthly cost estimate.
//!
//! Prices are read by hand from each provider's public page and committed with the date they
//! were read. The server never fetches a price. An estimate quotes its source and date and lists
//! every assumption; currencies are never converted (a rate would need a source of its own).

use super::{Figure, Tag, Traffic};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// A price older than this is still used, but the estimate says it is stale.
pub const STALE_AFTER_DAYS: i64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    /// A server billed by the month.
    Vm,
    /// A container billed while it runs (here: all month).
    Container,
    /// Billed per request and per GB-second.
    Function,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceEntry {
    pub id: String,
    pub provider: String,
    pub plan: String,
    pub kind: PlanKind,
    pub currency: String,
    /// The flat monthly price (0 for functions).
    pub monthly: f64,
    #[serde(default)]
    pub per_million_requests: Option<f64>,
    #[serde(default)]
    pub per_gb_second: Option<f64>,
    #[serde(default)]
    pub free_requests: Option<u64>,
    #[serde(default)]
    pub free_gb_seconds: Option<f64>,
    pub vcpu: u32,
    pub memory_mb: u32,
    #[serde(default)]
    pub included_egress_gb: Option<f64>,
    #[serde(default)]
    pub egress_per_gb: Option<f64>,
    pub source_url: String,
    pub as_of: NaiveDate,
    pub notes: String,
}

#[derive(Deserialize)]
struct Table {
    prices: Vec<PriceEntry>,
}

/// Every priced plan.
pub fn table() -> &'static [PriceEntry] {
    static TABLE: OnceLock<Vec<PriceEntry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str::<Table>(include_str!("prices.json")).expect("prices.json is valid (checked by its test)").prices
    })
}

pub fn find(id: &str) -> Option<&'static PriceEntry> {
    table().iter().find(|p| p.id == id)
}

/// Problems with the table itself: every entry needs an https source, a past date, sane numbers.
pub fn check_table(entries: &[PriceEntry], today: NaiveDate) -> Vec<String> {
    let mut problems = Vec::new();
    let mut ids = Vec::new();
    for p in entries {
        if ids.contains(&p.id.as_str()) {
            problems.push(format!("{}: duplicate id", p.id));
        }
        ids.push(p.id.as_str());
        if !super::is_slug(&p.id, 64) {
            problems.push(format!("{}: id is not a slug", p.id));
        }
        if url::Url::parse(&p.source_url).map(|u| u.scheme() != "https" || u.host_str().is_none()).unwrap_or(true) {
            problems.push(format!("{}: source_url must be an https address", p.id));
        }
        if p.as_of > today {
            problems.push(format!("{}: as_of is in the future", p.id));
        }
        if p.currency.len() != 3 || !p.currency.chars().all(|c| c.is_ascii_uppercase()) {
            problems.push(format!("{}: currency must be a 3-letter code", p.id));
        }
        let numbers = [Some(p.monthly), p.per_million_requests, p.per_gb_second, p.free_gb_seconds, p.included_egress_gb, p.egress_per_gb];
        if numbers.iter().flatten().any(|v| !v.is_finite() || *v < 0.0) {
            problems.push(format!("{}: prices must be numbers from 0", p.id));
        }
        if p.kind == PlanKind::Function && (p.per_million_requests.is_none() || p.per_gb_second.is_none()) {
            problems.push(format!("{}: a function plan needs per_million_requests and per_gb_second", p.id));
        }
    }
    problems
}

/// The monthly cost of running on `p` under `traffic`, as an estimate with its assumptions.
pub fn estimate_monthly(p: &PriceEntry, traffic: &Traffic, today: NaiveDate) -> Figure {
    let mut assumptions = vec![format!("{} {} ({}), price read {}", p.provider, p.plan, p.notes, p.as_of)];
    let egress_gb = traffic.monthly_requests as f64 * traffic.avg_response_kb / (1024.0 * 1024.0);
    let mut total = p.monthly;
    match p.kind {
        PlanKind::Vm | PlanKind::Container => {
            assumptions.push(format!("one instance running the whole month: {:.2} {}", p.monthly, p.currency));
            let included = p.included_egress_gb.unwrap_or(0.0);
            let over = (egress_gb - included).max(0.0);
            match p.egress_per_gb {
                Some(rate) => {
                    total += over * rate;
                    assumptions.push(format!(
                        "egress {egress_gb:.1} GB ({} requests × {} KB), {included} GB included, {rate} {}/GB above",
                        traffic.monthly_requests, traffic.avg_response_kb, p.currency
                    ));
                }
                None if over > 0.0 => assumptions.push(format!("egress {egress_gb:.1} GB is above the {included} GB included; overage not priced")),
                None => assumptions.push(format!("egress {egress_gb:.1} GB fits the {included} GB included")),
            }
            assumptions.push(format!("{} vCPU / {} MB is enough for the load (not checked here)", p.vcpu, p.memory_mb));
        }
        PlanKind::Function => {
            let rate_req = p.per_million_requests.unwrap_or(0.0);
            let rate_gbs = p.per_gb_second.unwrap_or(0.0);
            let billed_req = traffic.monthly_requests.saturating_sub(p.free_requests.unwrap_or(0)) as f64;
            let gb_seconds = traffic.monthly_requests as f64 * (traffic.avg_duration_ms / 1000.0) * (p.memory_mb as f64 / 1024.0);
            let billed_gbs = (gb_seconds - p.free_gb_seconds.unwrap_or(0.0)).max(0.0);
            total += billed_req / 1e6 * rate_req + billed_gbs * rate_gbs;
            assumptions.push(format!(
                "{} requests × {} ms at {} MB = {gb_seconds:.0} GB-s; free tier {} requests and {} GB-s taken off",
                traffic.monthly_requests,
                traffic.avg_duration_ms,
                p.memory_mb,
                p.free_requests.unwrap_or(0),
                p.free_gb_seconds.unwrap_or(0.0)
            ));
            assumptions.push(format!("{rate_req} {} per million requests, {rate_gbs} {} per GB-s", p.currency, p.currency));
        }
    }
    let stale = (today - p.as_of).num_days() > STALE_AFTER_DAYS;
    if stale {
        assumptions.push(format!("the price is more than {STALE_AFTER_DAYS} days old"));
    }
    Figure {
        metric: "monthly_cost".into(),
        value: (total * 100.0).round() / 100.0,
        unit: p.currency.clone(),
        tag: Tag::Estimate { source_url: p.source_url.clone(), as_of: p.as_of, stale, assumptions },
    }
}

/// The cost figure for an option: an estimate when it names a plan, otherwise speculation.
pub fn cost_figure(plan: Option<&str>, traffic: &Traffic) -> Figure {
    match plan.and_then(find) {
        Some(p) => estimate_monthly(p, traffic, Utc::now().date_naive()),
        None => Figure {
            metric: "monthly_cost".into(),
            value: 0.0,
            unit: String::new(),
            tag: Tag::Speculation { note: "no priced plan chosen for this option".into() },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traffic(requests: u64) -> Traffic {
        Traffic { monthly_requests: requests, peak_rps: 10.0, avg_response_kb: 20.0, avg_duration_ms: 100.0 }
    }

    #[test]
    fn the_checked_in_table_is_sound() {
        let problems = check_table(table(), Utc::now().date_naive());
        assert!(problems.is_empty(), "{problems:?}");
        assert!(table().len() >= 5);
    }

    #[test]
    fn a_bad_entry_is_refused() {
        let mut p = find("do-basic-1gb").unwrap().clone();
        p.source_url = "http://example.com".into();
        p.as_of = NaiveDate::from_ymd_opt(2999, 1, 1).unwrap();
        p.monthly = -1.0;
        let problems = check_table(&[p.clone(), p], NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
        for needle in ["duplicate", "https", "future", "from 0"] {
            assert!(problems.iter().any(|m| m.contains(needle)), "{needle}: {problems:?}");
        }
    }

    #[test]
    fn a_vm_costs_its_monthly_price_while_egress_fits() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let f = estimate_monthly(find("do-basic-1gb").unwrap(), &traffic(1_000_000), today);
        assert_eq!(f.value, 6.0);
        assert_eq!(f.unit, "USD");
        let Tag::Estimate { source_url, stale, assumptions, .. } = &f.tag else { panic!("{f:?}") };
        assert_eq!(source_url, "https://www.digitalocean.com/pricing/droplets");
        assert!(!stale);
        assert!(assumptions.iter().any(|a| a.contains("fits")), "{assumptions:?}");
    }

    #[test]
    fn egress_above_the_included_amount_is_priced_where_a_rate_is_known() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        // 10 M × 20 KB ≈ 190.7 GB at $0.02 = $3.81, plus $3.69
        let f = estimate_monthly(find("fly-shared-1x-512").unwrap(), &traffic(10_000_000), today);
        assert_eq!(f.value, 7.5);
    }

    #[test]
    fn a_function_bills_requests_and_gb_seconds_above_the_free_tier() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        // 10 M requests: 9 M billed = $1.80; 10 M × 0.1 s × 0.5 GB = 500 000 GB-s, 100 000 billed = $1.67
        let f = estimate_monthly(find("aws-lambda-x86-use1").unwrap(), &traffic(10_000_000), today);
        assert_eq!(f.value, 3.47);
        // inside the free tier it costs nothing
        assert_eq!(estimate_monthly(find("aws-lambda-x86-use1").unwrap(), &traffic(100_000), today).value, 0.0);
    }

    #[test]
    fn an_old_price_is_marked_stale_and_no_plan_is_speculation() {
        let later = NaiveDate::from_ymd_opt(2027, 6, 1).unwrap();
        let f = estimate_monthly(find("do-basic-1gb").unwrap(), &traffic(1), later);
        assert!(matches!(f.tag, Tag::Estimate { stale: true, .. }), "{f:?}");
        assert!(matches!(cost_figure(None, &traffic(1)).tag, Tag::Speculation { .. }));
        assert!(matches!(cost_figure(Some("nope"), &traffic(1)).tag, Tag::Speculation { .. }));
    }

    #[test]
    fn currencies_are_never_converted() {
        let f = cost_figure(Some("hetzner-cpx02"), &traffic(1000));
        assert_eq!((f.value, f.unit.as_str()), (6.49, "EUR"));
    }
}
