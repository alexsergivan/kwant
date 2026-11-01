//! Routing eval: feeds the cases in tests/router_eval/cases.json through the team router, without
//! a live Laya (a case may carry a fixed Laya guess), and reports the misroute rate per space.
//!
//! * `router_eval_cases_are_well_formed` runs in CI: the data parses and every name resolves.
//! * `router_eval` is `#[ignore]`d, because expected values for routes that don't exist yet
//!   (`concierge`, `ask`) fail by design. Run it on a branch with:
//!
//!   cargo test --test router_eval_test router_eval -- --ignored --nocapture
//!
//!   Set `ROUTER_EVAL_MAX_MISROUTE=0.05` to make it fail above a misroute rate (default: report only).
//!   Set `ROUTER_EVAL_CASES=/path/cases.json` to use another file.
//!
//! A *misroute* is a wrong destination. A *basis mismatch* is the right destination for the wrong
//! stated reason (basis); it is reported apart, not counted as a misroute.
//!
//! When the router's signature changes (a channel kind, a Concierge), only `route_case` needs to change.

use kwnt::db::models::Kwant;
use kwnt::router::team::{pick_member_after, LayaGuess};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct File {
    spaces: BTreeMap<String, Space>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Space {
    lead: String,
    members: Vec<Member>,
}
#[derive(Deserialize)]
struct Member {
    name: String,
    title: String,
    persona: String,
}
#[derive(Deserialize)]
struct Case {
    id: u32,
    space: String,
    channel: String,
    message: String,
    last_speaker: Option<String>,
    laya: Option<LayaCase>,
    expect: Expect,
}
#[derive(Deserialize)]
struct LayaCase {
    pick: String,
    confident: bool,
}
#[derive(Deserialize)]
struct Expect {
    route: String,
    #[serde(default)]
    basis: Vec<String>,
    #[serde(default)]
    candidates: Vec<String>,
}

fn load() -> File {
    let path = std::env::var("ROUTER_EVAL_CASES")
        .unwrap_or_else(|_| format!("{}/tests/router_eval/cases.json", env!("CARGO_MANIFEST_DIR")));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("cases.json parses")
}

fn kwants(space: &Space) -> Vec<Kwant> {
    space
        .members
        .iter()
        .map(|m| Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
            id: format!("id-{}", m.name),
            name: m.name.clone(),
            title: m.title.clone(),
            persona_prompt: m.persona.clone(),
            soul: String::new(),
            avatar_sprite: "x".into(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
            temperature: 0.7,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
        })
        .collect()
}

/// (destination, basis). Destination is a member name, `concierge` or `ask`.
fn route_case(case: &Case, space: &Space, members: &[Kwant]) -> (String, String) {
    let by_name = |n: &str| members.iter().find(|m| m.name == n).unwrap_or_else(|| panic!("case {}: no member {n}", case.id));
    let laya = case.laya.as_ref().map(|l| LayaGuess { kwant: by_name(&l.pick), confident: l.confident });
    let last = case.last_speaker.as_ref().map(|n| by_name(n).id.clone());
    // main: no channel kind and no Concierge yet, so `case.channel` is not passed.
    let pick = pick_member_after(&case.message, &by_name(&space.lead).id, members, laya, "channel lead", last.as_deref())
        .expect("a pick");
    let dest = match pick.basis {
        "concierge" => "concierge".to_string(),
        "ask" => "ask".to_string(),
        _ => pick.kwant.name.clone(),
    };
    (dest, pick.basis.to_string())
}

#[test]
fn router_eval_cases_are_well_formed() {
    let f = load();
    assert!(f.cases.len() >= 40, "at least 40 cases");
    let mut ids = std::collections::BTreeSet::new();
    for c in &f.cases {
        assert!(ids.insert(c.id), "duplicate case id {}", c.id);
        let s = f.spaces.get(&c.space).unwrap_or_else(|| panic!("case {}: unknown space {}", c.id, c.space));
        let has = |n: &str| s.members.iter().any(|m| m.name == n);
        assert!(has(&s.lead), "space {}: lead {} is not a member", c.space, s.lead);
        for n in c.last_speaker.iter().chain(c.laya.iter().map(|l| &l.pick)).chain(&c.expect.candidates) {
            assert!(has(n), "case {}: {n} is not in {}", c.id, c.space);
        }
        let r = &c.expect.route;
        assert!(r == "concierge" || r == "ask" || has(r), "case {}: bad route {r}", c.id);
    }
    assert!(f.cases.iter().filter(|c| c.expect.route == "concierge").count() >= 10, "about 10 no-match cases");
}

#[test]
#[ignore = "expected concierge/ask routes fail until the router overhaul lands; run with --ignored"]
fn router_eval() {
    let f = load();
    // space -> (total, misroutes, basis mismatches)
    let mut per: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    for c in &f.cases {
        let space = &f.spaces[&c.space];
        let members = kwants(space);
        let (dest, basis) = route_case(c, space, &members);
        let e = per.entry(c.space.clone()).or_default();
        e.0 += 1;
        let route_ok = dest == c.expect.route;
        let basis_ok = c.expect.basis.is_empty() || c.expect.basis.contains(&basis);
        if !route_ok {
            e.1 += 1;
            failures.push(format!("MISROUTE  #{:<2} [{}/{}] {:?}\n            expected {} {:?}, got {} (basis {})", c.id, c.space, c.channel, c.message, c.expect.route, c.expect.basis, dest, basis));
        } else if !basis_ok {
            e.2 += 1;
            failures.push(format!("BASIS     #{:<2} [{}/{}] {:?}\n            routed to {} as expected, but basis {} not in {:?}", c.id, c.space, c.channel, c.message, dest, basis, c.expect.basis));
        }
    }
    let (t, m, b) = per.values().fold((0, 0, 0), |a, v| (a.0 + v.0, a.1 + v.1, a.2 + v.2));
    println!("\nrouter eval: {} cases from {} spaces", t, per.len());
    for (s, (t, m, b)) in &per {
        println!("  {s:<9} {m:>2}/{t:<2} misrouted ({:>5.1}%), {b} basis mismatches", 100.0 * *m as f64 / *t as f64);
    }
    let no_match: Vec<&Case> = f.cases.iter().filter(|c| c.expect.route == "concierge").collect();
    let no_match_bad = no_match.iter().filter(|c| failures.iter().any(|x| x.starts_with("MISROUTE") && x.contains(&format!("#{:<2} ", c.id)))).count();
    println!("  TOTAL     {m:>2}/{t:<2} misrouted ({:>5.1}%), {b} basis mismatches", 100.0 * m as f64 / t as f64);
    println!("  no-match cases (expect concierge): {no_match_bad}/{} misrouted", no_match.len());
    println!("\nfailures:");
    for x in &failures {
        println!("  {x}");
    }
    if let Ok(max) = std::env::var("ROUTER_EVAL_MAX_MISROUTE") {
        let max: f64 = max.parse().expect("ROUTER_EVAL_MAX_MISROUTE is a number like 0.05");
        assert!(m as f64 / t as f64 <= max, "misroute rate {:.3} above {max}", m as f64 / t as f64);
    }
}
