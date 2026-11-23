//! pocket.json v1 for the Quiz kit (schema: autonomy/pocket-apps/pa2/schema.md).
//!
//! Every object refuses unknown fields, so a field the kit doesn't know (an `audio` clip, a typo)
//! is an error rather than silently dropped. Fields the validator checks for content are optional
//! here, so a missing one gets a plain numbered message instead of a parser error.

use serde::{Deserialize, Serialize};

/// The only version this kit reads.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuizSpec {
    /// Must be 1.
    #[serde(default)]
    pub version: Option<u32>,
    /// Must be "quiz" when present (pa1 stores the kit id beside the spec too).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    #[serde(default)]
    pub title: String,
    /// When it starts, RFC 3339 with an offset ("2026-11-27T20:00:00+00:00"); pa7 schedules it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Theme colour, "#rrggbb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default)]
    pub teams: Teams,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub lines: HostLines,
    #[serde(default)]
    pub rounds: Vec<Round>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMode {
    #[default]
    Solo,
    Teams,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Teams {
    #[serde(default)]
    pub mode: TeamMode,
    /// Most players a team (teams only), 2 to 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// Team names written ahead; when present, players pick one of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardEvery {
    #[default]
    Round,
    Question,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// When the leaderboard shows: after each round (default) or after each question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaderboard: Option<LeaderboardEvery>,
    /// Shuffle the options of choice questions once per game (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle_options: Option<bool>,
    /// Lock a question as soon as every connected player has answered (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_when_all_answered: Option<bool>,
    /// When set, locked, reveal and leaderboard screens move on by themselves after this many
    /// seconds (3 to 120). Unset: the host taps Next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_advance_s: Option<u32>,
    /// Lower the kit's player cap (1 to 60).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_players: Option<u32>,
}

impl Settings {
    pub fn leaderboard(&self) -> LeaderboardEvery {
        self.leaderboard.unwrap_or_default()
    }
    pub fn shuffle_options(&self) -> bool {
        self.shuffle_options.unwrap_or(true)
    }
    pub fn lock_when_all_answered(&self) -> bool {
        self.lock_when_all_answered.unwrap_or(true)
    }
}

/// The host's lines, written ahead so the live game never waits on a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostLines {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    /// Said at a reveal when the question has no `reveal_line` of its own (used in turn).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal: Vec<String>,
    #[serde(default, rename = "final", skip_serializing_if = "Option::is_none")]
    pub finale: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Round {
    #[serde(default)]
    pub title: String,
    /// The host's line for the start of the round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default)]
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Choice,
    Text,
    Number,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Choice => "choice",
            Kind::Text => "text",
            Kind::Number => "number",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub text: String,
    /// Choice only: 2 to 6 options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// Choice: the right option's text, exactly as in `options`. Text: the answer. Number: a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<serde_json::Value>,
    /// Text only: other spellings that also score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Vec<String>>,
    /// Number only: how far off still scores (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
    /// 1 to 100 (default 1). A `partial` verdict gives half, rounded down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    /// Seconds to answer, 5 to 300 (default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit_s: Option<u32>,
    /// Where the answer was checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<Picture>,
    /// The host's line at this question's reveal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal_line: Option<String>,
}

pub const DEFAULT_POINTS: u32 = 1;
pub const DEFAULT_TIME_LIMIT_S: u32 = 20;

impl Question {
    pub fn points(&self) -> u32 {
        self.points.unwrap_or(DEFAULT_POINTS)
    }
    pub fn time_limit_s(&self) -> u32 {
        self.time_limit_s.unwrap_or(DEFAULT_TIME_LIMIT_S)
    }
    pub fn tolerance(&self) -> f64 {
        self.tolerance.unwrap_or(0.0)
    }
    pub fn answer_text(&self) -> &str {
        self.answer.as_ref().and_then(|a| a.as_str()).unwrap_or("")
    }
    pub fn answer_number(&self) -> Option<f64> {
        self.answer.as_ref().and_then(|a| a.as_f64())
    }
    pub fn accepted(&self) -> &[String] {
        self.accept.as_deref().unwrap_or(&[])
    }
    pub fn options(&self) -> &[String] {
        self.options.as_deref().unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// The line on that page that holds the answer, quoted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quote: String,
    /// pa4c: the person wrote or changed this question themselves (`{"own": true}`), so it has
    /// no page and isn't source-checked. Only the person's draft edit API sets it; Quizmaster
    /// takes it as the person's only for a question the server recorded (`quiz_drafts.own_keys`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
}

impl Source {
    /// The person's own answer: no page, no quote.
    pub fn own() -> Self {
        Self { url: String::new(), quote: String::new(), own: true }
    }
}

/// The licences a picture may have (plan section 8 rule 8).
pub const LICENCES: &[&str] = &["own", "public-domain", "cc0", "cc-by"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picture {
    /// Where it comes from: https, or for the host's own photo a path in the bot's files. The box
    /// serves the picture itself; guests never load an outside address.
    #[serde(default)]
    pub url: String,
    /// One of `own`, `public-domain`, `cc0`, `cc-by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    /// Who made it, shown with the picture (needed unless the licence is `own`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    /// Words for people who can't see it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}
