//! The budget line (pa3 brief item 6, pa0a design "Budget line"): before a pocket is shared, the
//! card says what it used and what live play may use, counted from real calls.
//!
//! - writing: one bot call per quiz the bot hands in (each `pocket_create` / `pocket_update`
//!   with a spec is the end of one writing turn);
//! - source checks fetch pages and use no model, so they cost nothing on the account;
//! - judging: at most [`JUDGE_CALLS_PER_TEXT_QUESTION`] calls per text question (at least
//!   [`MIN_JUDGE_CALLS`]); that allowance is also the judge's hard cap ([`super::judge::JudgeBudget`]).
//!
//! Dollars are estimates from the per-call figures below, always written "about".

use serde::{Deserialize, Serialize};

/// About what one writing call costs: a quiz of ~40 questions is ~6k output tokens on a mid model.
pub const USD_PER_WRITING_CALL: f64 = 0.03;
/// About what one judge call costs: ~400 input tokens and one output word on a fast model.
pub const USD_PER_JUDGE_CALL: f64 = 0.0005;
pub const JUDGE_CALLS_PER_TEXT_QUESTION: u32 = 4;
pub const MIN_JUDGE_CALLS: u32 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Quizzes the bot handed in.
    pub writing_calls: u32,
    /// Pages fetched for the source check (no model calls).
    pub source_fetches: u32,
    /// Judge calls made in live play.
    #[serde(default)]
    pub judge_calls: u32,
    /// The judge's cap for this quiz.
    pub judge_allowance: u32,
}

/// The judge allowance for a quiz with this many text questions.
pub fn judge_allowance(text_questions: usize) -> u32 {
    (text_questions as u32).saturating_mul(JUDGE_CALLS_PER_TEXT_QUESTION).max(MIN_JUDGE_CALLS)
}

fn dollars(x: f64) -> String {
    if x > 0.0 && x < 0.01 {
        "less than $0.01".into()
    } else {
        format!("about ${x:.2}")
    }
}

impl Budget {
    pub fn bot_calls(&self) -> u32 {
        self.writing_calls + self.judge_calls
    }

    pub fn usd(&self) -> f64 {
        f64::from(self.writing_calls) * USD_PER_WRITING_CALL + f64::from(self.judge_calls) * USD_PER_JUDGE_CALL
    }

    /// The card's footer line.
    pub fn line(&self) -> String {
        let n = self.bot_calls();
        let calls = if n == 1 { "1 bot call".to_string() } else { format!("{n} bot calls") };
        let cap = f64::from(self.judge_allowance) * USD_PER_JUDGE_CALL;
        format!(
            "About {calls} so far, {} on your account. Live play uses none unless a near miss needs judging: at most {} judge calls, {}.",
            dollars(self.usd()),
            self.judge_allowance,
            dollars(cap),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_counts_real_calls_and_states_the_judge_cap() {
        let b = Budget { writing_calls: 2, source_fetches: 40, judge_calls: 0, judge_allowance: judge_allowance(10) };
        assert_eq!(b.bot_calls(), 2, "page fetches are not bot calls");
        assert_eq!(
            b.line(),
            "About 2 bot calls so far, about $0.06 on your account. Live play uses none unless a near miss needs judging: at most 40 judge calls, about $0.02."
        );
        let one = Budget { writing_calls: 1, judge_allowance: judge_allowance(0), ..Default::default() };
        assert!(one.line().starts_with("About 1 bot call so far"));
        assert!(one.line().ends_with("at most 10 judge calls, less than $0.01."), "{}", one.line());
        assert_eq!(judge_allowance(1), MIN_JUDGE_CALLS);
    }
}
