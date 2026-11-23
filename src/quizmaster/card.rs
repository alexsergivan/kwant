//! Readiness rules pa3 adds on top of the Quiz validator, and the Pocket card a bot shows
//! (pa0a design: Writing, Checking, Dry run stopped, Ready; then Open and Closed).

use super::budget::Budget;
use super::source_check::SourceResult;
use crate::kits::quiz::spec::{Kind, QuizSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The spec doesn't validate yet, or misses something pa3 needs.
    Writing,
    /// Some answers didn't match their source: those questions need replacing.
    Checking,
    /// Every answer is checked, but the dry run found a problem.
    DryRunStopped,
    /// N/N checked and the dry run passed: the pocket may open.
    Ready,
    Open,
    Closed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Writing => "writing",
            State::Checking => "checking",
            State::DryRunStopped => "dry_run_stopped",
            State::Ready => "ready",
            State::Open => "open",
            State::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [State::Writing, State::Checking, State::DryRunStopped, State::Ready, State::Open, State::Closed].into_iter().find(|x| x.as_str() == s)
    }
    /// The chip the card shows.
    pub fn chip(self) -> &'static str {
        match self {
            State::Writing => "Writing",
            State::Checking => "Checking",
            State::DryRunStopped => "Dry run stopped",
            State::Ready => "Ready",
            State::Open => "Open",
            State::Closed => "Closed",
        }
    }
}

/// What pa3 needs beyond a valid spec, as fix-it lines: lines written ahead, and pictures only
/// from the host's own files (pa3/notes.md item 6) until pa1/pa5 serve copies.
pub fn rule_problems(spec: &QuizSpec) -> Vec<String> {
    let mut out = Vec::new();
    let blank = |s: &Option<String>| s.as_deref().is_none_or(|t| t.trim().is_empty());
    if blank(&spec.lines.intro) {
        out.push("lines: add `lines.intro`, the host's opening line (every line is written now, so the show never waits on a model)".to_string());
    }
    if blank(&spec.lines.finale) {
        out.push("lines: add `lines.final`, the host's closing line".to_string());
    }
    let every_q_has_reveal = spec.rounds.iter().flat_map(|r| &r.questions).all(|q| !blank(&q.reveal_line));
    if spec.lines.reveal.iter().all(|l| l.trim().is_empty()) && !every_q_has_reveal {
        out.push("lines: add at least one `lines.reveal` line (said at a reveal when a question has no `reveal_line`)".to_string());
    }
    for (r, round) in spec.rounds.iter().enumerate() {
        for (i, q) in round.questions.iter().enumerate() {
            if let Some(p) = &q.picture {
                let own = p.licence.as_deref() == Some("own") && p.url.trim().starts_with("files/");
                if !own {
                    out.push(format!(
                        "round {}, question {}: pictures from the web are off for now; only the host's own photos (`licence: \"own\"` and a `files/…` path) can be used, so remove this picture",
                        r + 1,
                        i + 1
                    ));
                }
            }
        }
    }
    out
}

/// Advice that doesn't stop the quiz: a 1-point text question can't earn partial credit
/// (half of 1, rounded down, is 0), so text questions where half-right is likely get 2 points.
pub fn rule_notes(spec: &QuizSpec) -> Vec<String> {
    let mut out = Vec::new();
    for (r, round) in spec.rounds.iter().enumerate() {
        for (i, q) in round.questions.iter().enumerate() {
            if q.kind == Some(Kind::Text) && q.points() < 2 {
                out.push(format!(
                    "round {}, question {}: a text question worth 1 point can't give half points; make it 2 if a half-right answer (a surname only) is likely",
                    r + 1,
                    i + 1
                ));
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunLine {
    pub ok: bool,
    pub summary: String,
    #[serde(default)]
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailedSource {
    pub q: usize,
    pub at: String,
    pub url: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinesStatus {
    pub intro: bool,
    pub reveal: usize,
    #[serde(rename = "final")]
    pub finale: bool,
    /// Questions with their own reveal line.
    pub question_lines: usize,
}

/// The Pocket card a bot shows in chat (pa4 draws it; pa3 fills it).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub id: String,
    pub kit: &'static str,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub state: State,
    pub chip: &'static str,
    pub rounds: usize,
    pub questions: usize,
    pub checked: usize,
    pub total: usize,
    pub failed: Vec<FailedSource>,
    pub problems: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<DryRunLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<LinesStatus>,
    pub notes: Vec<String>,
    pub budget: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pocket: Option<String>,
    /// What the bot does next.
    pub next: String,
}

pub struct CardInput<'a> {
    pub id: &'a str,
    pub state: State,
    pub spec: Option<&'a QuizSpec>,
    pub title: &'a str,
    pub problems: &'a [String],
    pub sources: &'a [SourceResult],
    pub dry_run: Option<&'a DryRunLine>,
    pub budget: &'a Budget,
    pub pocket: Option<&'a str>,
}

pub fn build(i: CardInput<'_>) -> Card {
    let total = i.spec.map(|s| s.rounds.iter().map(|r| r.questions.len()).sum()).unwrap_or(0);
    let checked = i.sources.iter().filter(|r| r.ok).count();
    let failed: Vec<FailedSource> = i
        .sources
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            let why = r.fail.map(|f| f.says().to_string()).unwrap_or_default();
            FailedSource { q: r.q, at: r.at.clone(), url: r.url.clone(), why: match &r.detail { Some(d) => format!("{why} ({d})"), None => why } }
        })
        .collect();
    let lines = i.spec.map(|s| LinesStatus {
        intro: s.lines.intro.as_deref().is_some_and(|t| !t.trim().is_empty()),
        reveal: s.lines.reveal.iter().filter(|l| !l.trim().is_empty()).count(),
        finale: s.lines.finale.as_deref().is_some_and(|t| !t.trim().is_empty()),
        question_lines: s.rounds.iter().flat_map(|r| &r.questions).filter(|q| q.reveal_line.as_deref().is_some_and(|t| !t.trim().is_empty())).count(),
    });
    let next = match i.state {
        State::Writing => "Fix every numbered problem at once, then call pocket_update with this id and the whole spec.".to_string(),
        State::Checking => format!(
            "Replace the {} question(s) listed under \"Not checked\" with new ones whose answer is in a quote from an https page (or fix the source), then call pocket_update with the whole spec. Don't show the person this card yet.",
            failed.len()
        ),
        State::DryRunStopped => "Fix what the dry run found, then call pocket_update with the whole spec.".to_string(),
        State::Ready => "Show the person this card. Call pocket_open when they want guests to join; near misses are judged in seconds, and the host can overrule.".to_string(),
        State::Open => "Guests can join. Call pocket_close when the quiz is over.".to_string(),
        State::Closed => "This quiz is closed.".to_string(),
    };
    Card {
        id: i.id.to_string(),
        kit: "quiz",
        title: i.spec.map(|s| s.title.trim().to_string()).unwrap_or_else(|| i.title.to_string()),
        when: i.spec.and_then(|s| s.when.clone()),
        state: i.state,
        chip: i.state.chip(),
        rounds: i.spec.map(|s| s.rounds.len()).unwrap_or(0),
        questions: total,
        checked,
        total: if i.sources.is_empty() { total } else { i.sources.len() },
        failed,
        problems: i.problems.to_vec(),
        dry_run: i.dry_run.cloned(),
        lines,
        notes: i.spec.map(rule_notes).unwrap_or_default(),
        budget: i.budget.line(),
        pocket: i.pocket.map(str::to_string),
        next,
    }
}

impl Card {
    /// The card as text for the bot, then as JSON.
    pub fn text(&self) -> String {
        let mut out = format!("Pocket app · Quiz [{}] \"{}\"", self.id, if self.title.is_empty() { "untitled" } else { &self.title });
        if let Some(w) = &self.when {
            out.push_str(&format!(" · {w}"));
        }
        out.push_str(&format!(" · {}\n", self.chip));
        if self.questions > 0 {
            out.push_str(&format!("{} questions in {} rounds.\n", self.questions, self.rounds));
        }
        if !self.problems.is_empty() {
            out.push_str("Problems:\n");
            for p in &self.problems {
                out.push_str(&format!("  {p}\n"));
            }
        }
        if self.total > 0 && self.state != State::Writing || !self.failed.is_empty() {
            out.push_str(&format!("{}/{} answers checked against a source.\n", self.checked, self.total));
        }
        if !self.failed.is_empty() {
            out.push_str("Not checked:\n");
            for f in &self.failed {
                out.push_str(&format!("  Q{} ({}): {} — {}\n", f.q + 1, f.at, f.why, f.url));
            }
        }
        if let Some(d) = &self.dry_run {
            out.push_str(&format!("{}\n", d.summary));
            for p in &d.problems {
                out.push_str(&format!("  {p}\n"));
            }
        }
        if let Some(l) = &self.lines {
            out.push_str(&format!(
                "Lines written ahead: intro {}, {} reveal line(s), {} question line(s), final {}.\n",
                if l.intro { "yes" } else { "missing" },
                l.reveal,
                l.question_lines,
                if l.finale { "yes" } else { "missing" }
            ));
        }
        for n in &self.notes {
            out.push_str(&format!("Note: {n}\n"));
        }
        if let Some(p) = &self.pocket {
            out.push_str(&format!("Pocket: {p}\n"));
        }
        out.push_str(&format!("{}\nNext: {}\n", self.budget, self.next));
        out.push_str(&format!("card: {}", serde_json::to_string(self).unwrap_or_default()));
        out
    }
}
