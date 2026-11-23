//! Quizmaster (pa3, autonomy/pocket-apps/pa3): turns "make a pub-quiz app for Friday" into a
//! Quiz pocket that is checked before anyone sees it, and judges near misses strictly in play.
//!
//! - [`evaluate`]: validate (pa2), pa3's readiness rules ([`card::rule_problems`]), the source
//!   check ([`source_check`]) and the dry run, into one [`card::Card`] state;
//! - [`judge`]: the strict judge for live play;
//! - [`budget`]: the budget line;
//! - [`preset`]: the Quizmaster preset, its skill text and preset kits.
//!
//! Behind the `pocket_apps` switch: the bot tools in `server::quizmaster_tools` are offered only
//! while it is on.

pub mod budget;
pub mod card;
pub mod judge;
pub mod preset;
pub mod source_check;

use crate::kits::quiz::{spec::QuizSpec, validate, QuizKit};
use crate::kits::Kit;
use card::{DryRunLine, State};
use source_check::{Policy, SourceResult};
use std::collections::HashMap;

/// Pretend players in the dry run, and how many drop and rejoin (plan section 10, pa3).
pub const DRY_RUN_PLAYERS: usize = 30;
pub const DRY_RUN_DROP: usize = 5;
pub const MAX_DRY_RUN_PLAYERS: usize = 60;

/// The result of checking a spec.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub spec: Option<QuizSpec>,
    pub state: State,
    /// Numbered fix-it lines: the validator's, then pa3's.
    pub problems: Vec<String>,
    pub sources: Vec<SourceResult>,
    pub dry_run: Option<DryRunLine>,
    /// Pages fetched this time (cached results don't count).
    pub fetched: u32,
}

/// The dry run's seed: fixed per spec, so the same quiz gives the same report.
fn seed(spec_json: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(spec_json.as_bytes());
    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
}

/// Runs the Quiz kit's dry run and keeps the card's part of the report.
pub fn dry_run(spec_json: &str, players: usize) -> Result<DryRunLine, Vec<String>> {
    let players = players.clamp(2, MAX_DRY_RUN_PLAYERS);
    let report = QuizKit.dry_run(spec_json, players, DRY_RUN_DROP.min(players / 2), seed(spec_json)).map_err(|p| p.iter().map(|p| p.to_string()).collect::<Vec<_>>())?;
    Ok(DryRunLine {
        ok: report["ok"].as_bool().unwrap_or(false),
        summary: report["summary"].as_str().unwrap_or("Dry run: no report").to_string(),
        problems: report["problems"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default(),
    })
}

/// Checks a spec end to end. Source results whose question didn't change come from `cache`.
pub async fn evaluate(spec_json: &str, cache: &HashMap<String, SourceResult>, policy: &Policy) -> Evaluation {
    let spec = match validate::parse(spec_json) {
        Ok(s) => s,
        Err(problems) => {
            return Evaluation { spec: None, state: State::Writing, problems: problems.iter().map(|p| p.to_string()).collect(), sources: vec![], dry_run: None, fetched: 0 }
        }
    };
    let problems: Vec<String> = card::rule_problems(&spec).into_iter().enumerate().map(|(i, p)| format!("{}. {p}", i + 1)).collect();
    let sources = source_check::check_spec(&spec, policy, cache).await;
    let fetched = sources.iter().filter(|r| !cache.contains_key(&r.key) && r.fail != Some(source_check::Fail::NoSource)).count() as u32;
    let dry = dry_run(spec_json, DRY_RUN_PLAYERS).ok();
    let state = if !problems.is_empty() {
        State::Writing
    } else if sources.iter().any(|r| !r.ok) {
        State::Checking
    } else if !dry.as_ref().is_some_and(|d| d.ok) {
        State::DryRunStopped
    } else {
        State::Ready
    };
    Evaluation { spec: Some(spec), state, problems, sources, dry_run: dry, fetched }
}

/// Text questions in a spec (the judge allowance counts them).
pub fn text_questions(spec: &QuizSpec) -> usize {
    spec.rounds.iter().flat_map(|r| &r.questions).filter(|q| q.kind == Some(crate::kits::quiz::spec::Kind::Text)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::quiz::EXAMPLE;

    #[tokio::test]
    async fn a_bad_spec_stays_writing_with_every_problem_numbered() {
        let e = evaluate(r#"{"version": 1, "kit": "quiz", "rounds": []}"#, &HashMap::new(), &Policy::default()).await;
        assert_eq!(e.state, State::Writing);
        assert!(e.problems.len() >= 2, "{:?}", e.problems);
        assert!(e.problems[0].starts_with("1. "));
        assert!(e.sources.is_empty(), "nothing is fetched before the spec validates");
    }

    #[tokio::test]
    async fn missing_lines_and_web_pictures_are_problems() {
        let mut v: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
        v["lines"] = serde_json::json!({});
        v["rounds"][1]["questions"][1]["picture"] = serde_json::json!({"url": "https://upload.example/tower.jpg", "licence": "cc0", "alt": "a tower"});
        let e = evaluate(&v.to_string(), &HashMap::new(), &Policy::default()).await;
        assert_eq!(e.state, State::Writing);
        let all = e.problems.join("\n");
        assert!(all.contains("lines.intro") && all.contains("lines.final") && all.contains("lines.reveal"), "{all}");
        assert!(all.contains("round 2, question 2: pictures from the web are off"), "{all}");
    }

    #[test]
    fn one_point_text_questions_get_a_note() {
        let spec: QuizSpec = serde_json::from_str(EXAMPLE).unwrap();
        let notes = card::rule_notes(&spec);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("round 2, question 2"), "the 2-point Zidane question has no note: {notes:?}");
    }

    #[test]
    fn example_dry_run_passes() {
        let d = dry_run(EXAMPLE, DRY_RUN_PLAYERS).unwrap();
        assert!(d.ok, "{d:?}");
        assert!(d.summary.contains("30"), "{}", d.summary);
    }
}
