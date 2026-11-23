//! The Quiz state machine: lobby -> question -> locked -> reveal -> (leaderboard) -> ... -> final.
//!
//! Driven by host actions and by [`QuizGame::tick`] with an injected "now". One answer per player
//! per question (a second is ignored), late answers refused. Scores are keyed by player id, so a
//! player who drops and rejoins keeps every point and every answer.
//!
//! A score only counts questions whose answer has been revealed, so a player can't learn from
//! their score whether an answer was right before the reveal.

use super::matching::{self, normalise, TextMatch};
use super::spec::{Kind, LeaderboardEvery, QuizSpec, TeamMode};
use super::validate::MAX_PLAYERS;
use crate::kits::rng::SeededRng;
use crate::kits::{ActionError, ActionOutcome, PlayerId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Longest answer a guest may send, in characters.
pub const MAX_GUEST_ANSWER: usize = 100;
/// Longest display name the kit keeps (pa1 filters names before they get here).
pub const MAX_NAME: usize = 20;
/// Largest +/- the host can give in one tap.
pub const MAX_ADJUST: i64 = 100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Lobby,
    Question,
    Locked,
    Reveal,
    Leaderboard,
    Final,
}

/// The strict verdict a judge (the bot hook or the host) gives a near miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeVerdict {
    Yes,
    No,
    Partial,
}

impl JudgeVerdict {
    /// Only these three words, exactly; anything else is refused.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            "partial" => Some(Self::Partial),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Verdict {
    Correct,
    Wrong,
    NeedsJudge,
    Judged { verdict: JudgeVerdict, by_host: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AnswerValue {
    /// The option's index in the spec (not its shuffled position).
    Choice(usize),
    Text(String),
    Number(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnswerRecord {
    pub player: PlayerId,
    pub value: AnswerValue,
    pub at: DateTime<Utc>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub team: Option<String>,
    pub connected: bool,
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Target {
    Player(PlayerId),
    Team(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjustment {
    pub target: Target,
    pub points: i64,
}

/// Where a flat question index sits in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QRef {
    pub round: usize,
    pub index: usize,
    pub first_in_round: bool,
    pub last_in_round: bool,
}

/// A near miss waiting for a verdict, as the judge hook (pa3) gets it. The guess is a guest's
/// words: pa3 frames it as an outsider's text (plan section 8 rules 1 and 4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JudgeRequest {
    pub q: usize,
    pub player: PlayerId,
    pub question: String,
    pub answer: String,
    pub accept: Vec<String>,
    pub guess: String,
}

/// One player's line on a leaderboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoreLine {
    pub player: PlayerId,
    pub name: String,
    pub team: Option<String>,
    pub score: i64,
    /// 1-based; equal scores share a rank (1, 1, 3).
    pub rank: usize,
    pub tied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamLine {
    pub team: String,
    pub members: usize,
    pub score: i64,
    pub rank: usize,
    pub tied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuizGame {
    #[serde(skip)]
    pub(crate) spec: QuizSpec,
    #[serde(skip)]
    pub(crate) qrefs: Vec<QRef>,
    pub(crate) seed: u64,
    /// Per question: shuffled position -> spec option index (empty for non-choice questions).
    pub(crate) option_order: Vec<Vec<usize>>,
    pub(crate) phase: Phase,
    pub(crate) current: usize,
    pub(crate) started: bool,
    pub(crate) deadline: Option<DateTime<Utc>>,
    pub(crate) paused_remaining_ms: Option<i64>,
    pub(crate) phase_since: DateTime<Utc>,
    pub(crate) players: Vec<Player>,
    pub(crate) answers: Vec<Vec<AnswerRecord>>,
    pub(crate) adjustments: Vec<Adjustment>,
    pub(crate) ended_at: Option<DateTime<Utc>>,
}

fn qrefs(spec: &QuizSpec) -> Vec<QRef> {
    let mut out = Vec::new();
    for (ri, r) in spec.rounds.iter().enumerate() {
        let n = r.questions.len();
        for qi in 0..n {
            out.push(QRef { round: ri, index: qi, first_in_round: qi == 0, last_in_round: qi + 1 == n });
        }
    }
    out
}

impl QuizGame {
    /// A new game in its lobby. The spec must already be valid ([`super::validate::parse`]).
    pub fn new(spec: QuizSpec, seed: u64, now: DateTime<Utc>) -> Self {
        let qrefs = qrefs(&spec);
        let mut rng = SeededRng::new(seed);
        let shuffle = spec.settings.shuffle_options();
        let option_order = qrefs
            .iter()
            .map(|r| {
                let q = &spec.rounds[r.round].questions[r.index];
                if q.kind != Some(Kind::Choice) {
                    return Vec::new();
                }
                let mut order: Vec<usize> = (0..q.options().len()).collect();
                if shuffle {
                    rng.shuffle(&mut order);
                }
                order
            })
            .collect();
        let n = qrefs.len();
        Self {
            spec,
            qrefs,
            seed,
            option_order,
            phase: Phase::Lobby,
            current: 0,
            started: false,
            deadline: None,
            paused_remaining_ms: None,
            phase_since: now,
            players: Vec::new(),
            answers: vec![Vec::new(); n],
            adjustments: Vec::new(),
            ended_at: None,
        }
    }

    /// Rebuilds a game from its spec and saved state.
    pub fn restore(spec: QuizSpec, saved: &serde_json::Value) -> Result<Self, String> {
        let mut g: QuizGame = serde_json::from_value(saved.clone()).map_err(|e| format!("saved quiz state doesn't read: {e}"))?;
        g.qrefs = qrefs(&spec);
        g.spec = spec;
        let n = g.qrefs.len();
        if g.answers.len() != n || g.option_order.len() != n || (n > 0 && g.current >= n) {
            return Err("saved quiz state doesn't match this spec's questions".into());
        }
        for (i, r) in g.qrefs.iter().enumerate() {
            let q = &g.spec.rounds[r.round].questions[r.index];
            let want = if q.kind == Some(Kind::Choice) { q.options().len() } else { 0 };
            if g.option_order[i].len() != want {
                return Err("saved quiz state doesn't match this spec's options".into());
            }
        }
        Ok(g)
    }

    pub fn save(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn spec(&self) -> &QuizSpec {
        &self.spec
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn current(&self) -> usize {
        self.current
    }
    pub fn total(&self) -> usize {
        self.qrefs.len()
    }
    pub fn players(&self) -> &[Player] {
        &self.players
    }
    pub fn deadline(&self) -> Option<DateTime<Utc>> {
        self.deadline
    }
    pub fn answers_for(&self, q: usize) -> &[AnswerRecord] {
        self.answers.get(q).map(Vec::as_slice).unwrap_or(&[])
    }
    pub(crate) fn qref(&self, q: usize) -> QRef {
        self.qrefs[q]
    }
    pub fn question(&self, q: usize) -> &super::spec::Question {
        let r = self.qrefs[q];
        &self.spec.rounds[r.round].questions[r.index]
    }
    pub(crate) fn option_order(&self, q: usize) -> &[usize] {
        &self.option_order[q]
    }
    pub(crate) fn max_players(&self) -> usize {
        self.spec.settings.max_players.unwrap_or(MAX_PLAYERS).min(MAX_PLAYERS) as usize
    }
    pub(crate) fn player(&self, id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }
    fn player_mut(&mut self, id: &str) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }
    fn active(&self, id: &str) -> Result<&Player, ActionError> {
        match self.player(id) {
            None => Err(ActionError::UnknownPlayer),
            Some(p) if p.removed => Err(ActionError::Removed),
            Some(p) => Ok(p),
        }
    }
    pub(crate) fn record(&self, q: usize, player: &str) -> Option<&AnswerRecord> {
        self.answers.get(q)?.iter().find(|a| a.player == player)
    }

    /// The spec index of a choice question's right option.
    pub fn right_option(&self, q: usize) -> Option<usize> {
        let qq = self.question(q);
        let ans = normalise(qq.answer_text());
        qq.options().iter().position(|o| normalise(o) == ans)
    }

    /// How many questions (from the first) have had their answer revealed.
    pub fn revealed_count(&self) -> usize {
        if !self.started {
            return 0;
        }
        match self.phase {
            Phase::Lobby => 0,
            Phase::Question | Phase::Locked => self.current,
            Phase::Reveal | Phase::Leaderboard | Phase::Final => self.current + 1,
        }
    }

    pub fn is_revealed(&self, q: usize) -> bool {
        q < self.revealed_count()
    }

    pub fn ended(&self) -> bool {
        self.phase == Phase::Final
    }

    // ---- joining --------------------------------------------------------------------------

    pub fn join(&mut self, id: &str, name: &str, _now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        if let Some(p) = self.player_mut(id) {
            if p.removed {
                return Err(ActionError::Removed);
            }
            p.connected = true;
            return Ok(ActionOutcome::Ignored("welcome back: your answers and points are kept".into()));
        }
        if self.phase == Phase::Final {
            return Err(ActionError::WrongPhase("the quiz has ended".into()));
        }
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > MAX_NAME {
            return Err(ActionError::BadInput(format!("a name is 1 to {MAX_NAME} characters")));
        }
        if id.is_empty() {
            return Err(ActionError::BadInput("a player needs an id".into()));
        }
        let count = self.players.iter().filter(|p| !p.removed).count();
        if count >= self.max_players() {
            return Err(ActionError::Full(format!("the quiz is full ({} players)", self.max_players())));
        }
        self.players.push(Player { id: id.to_string(), name, team: None, connected: true, removed: false });
        Ok(ActionOutcome::Applied)
    }

    pub fn set_connected(&mut self, id: &str, connected: bool) -> bool {
        match self.player_mut(id) {
            Some(p) if p.connected != connected => {
                p.connected = connected;
                true
            }
            _ => false,
        }
    }

    pub fn set_team(&mut self, id: &str, team: &str) -> Result<ActionOutcome, ActionError> {
        if self.spec.teams.mode != TeamMode::Teams {
            return Err(ActionError::NotAllowed("this quiz is played solo".into()));
        }
        let p = self.active(id)?;
        if p.team.is_some() && self.phase != Phase::Lobby {
            return Err(ActionError::WrongPhase("teams can only change in the lobby".into()));
        }
        let wanted = team.split_whitespace().collect::<Vec<_>>().join(" ");
        let norm = normalise(&wanted);
        if norm.is_empty() || wanted.chars().count() > super::validate::MAX_TEAM_NAME {
            return Err(ActionError::BadInput(format!("a team name is 1 to {} characters", super::validate::MAX_TEAM_NAME)));
        }
        // A name written ahead wins; otherwise an existing team with the same name (any case).
        let canonical = if let Some(names) = &self.spec.teams.names {
            match names.iter().find(|n| normalise(n) == norm) {
                Some(n) => n.trim().to_string(),
                None => return Err(ActionError::BadInput(format!("pick one of the teams: {}", names.join(", ")))),
            }
        } else {
            self.players
                .iter()
                .filter_map(|p| p.team.clone())
                .find(|t| normalise(t) == norm)
                .unwrap_or(wanted)
        };
        if p.team.as_deref() == Some(canonical.as_str()) {
            return Ok(ActionOutcome::Ignored("already in that team".into()));
        }
        let size = self.spec.teams.size.unwrap_or(u32::MAX) as usize;
        let members = self.players.iter().filter(|q| !q.removed && q.team.as_deref() == Some(canonical.as_str())).count();
        if members >= size {
            return Err(ActionError::Full(format!("{canonical} is full ({size} players)")));
        }
        if let Some(p) = self.player_mut(id) {
            p.team = Some(canonical);
        }
        Ok(ActionOutcome::Applied)
    }

    pub fn remove(&mut self, id: &str) -> Result<ActionOutcome, ActionError> {
        let p = self.player_mut(id).ok_or(ActionError::UnknownPlayer)?;
        if p.removed {
            return Ok(ActionOutcome::Ignored("already removed".into()));
        }
        p.removed = true;
        p.connected = false;
        Ok(ActionOutcome::Applied)
    }

    // ---- phases ---------------------------------------------------------------------------

    fn enter_question(&mut self, q: usize, at: DateTime<Utc>) {
        self.phase = Phase::Question;
        self.current = q;
        self.started = true;
        self.deadline = Some(at + Duration::seconds(i64::from(self.question(q).time_limit_s())));
        self.paused_remaining_ms = None;
        self.phase_since = at;
    }

    fn lock(&mut self, at: DateTime<Utc>) {
        self.phase = Phase::Locked;
        self.deadline = None;
        self.paused_remaining_ms = None;
        self.phase_since = at;
    }

    fn finish(&mut self, at: DateTime<Utc>) {
        self.phase = Phase::Final;
        self.deadline = None;
        self.paused_remaining_ms = None;
        self.phase_since = at;
        self.ended_at = Some(at);
    }

    fn leaderboard_due(&self) -> bool {
        let r = self.qrefs[self.current];
        r.last_in_round || self.spec.settings.leaderboard() == LeaderboardEvery::Question
    }

    fn last(&self) -> bool {
        self.current + 1 >= self.qrefs.len()
    }

    /// Next: the host's main button.
    pub fn next(&mut self, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        self.tick(now);
        self.advance(now)
    }

    fn advance(&mut self, at: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        match self.phase {
            Phase::Lobby => {
                if !self.players.iter().any(|p| !p.removed) {
                    return Err(ActionError::WrongPhase("nobody has joined yet".into()));
                }
                self.enter_question(0, at);
            }
            Phase::Question => self.lock(at),
            Phase::Locked => {
                self.phase = Phase::Reveal;
                self.phase_since = at;
            }
            Phase::Reveal => {
                if self.leaderboard_due() {
                    self.phase = Phase::Leaderboard;
                    self.phase_since = at;
                } else if self.last() {
                    self.finish(at);
                } else {
                    self.enter_question(self.current + 1, at);
                }
            }
            Phase::Leaderboard => {
                if self.last() {
                    self.finish(at);
                } else {
                    self.enter_question(self.current + 1, at);
                }
            }
            Phase::Final => return Err(ActionError::WrongPhase("the quiz has ended".into())),
        }
        Ok(ActionOutcome::Applied)
    }

    pub fn start(&mut self, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        if self.phase != Phase::Lobby {
            return Err(ActionError::WrongPhase("the quiz has already started".into()));
        }
        self.advance(now)
    }

    pub fn pause(&mut self, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        self.tick(now);
        match (self.phase, self.deadline) {
            (Phase::Question, Some(d)) => {
                self.paused_remaining_ms = Some((d - now).num_milliseconds().max(0));
                self.deadline = None;
                Ok(ActionOutcome::Applied)
            }
            (Phase::Question, None) => Ok(ActionOutcome::Ignored("already paused".into())),
            _ => Err(ActionError::WrongPhase("only a question's timer can pause".into())),
        }
    }

    pub fn resume(&mut self, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        match (self.phase, self.paused_remaining_ms) {
            (Phase::Question, Some(ms)) => {
                self.deadline = Some(now + Duration::milliseconds(ms));
                self.paused_remaining_ms = None;
                Ok(ActionOutcome::Applied)
            }
            (Phase::Question, None) => Ok(ActionOutcome::Ignored("not paused".into())),
            _ => Err(ActionError::WrongPhase("only a paused question can resume".into())),
        }
    }

    pub fn end(&mut self, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        if self.phase == Phase::Final {
            return Ok(ActionOutcome::Ignored("already ended".into()));
        }
        self.finish(now);
        Ok(ActionOutcome::Applied)
    }

    /// Runs timers up to `now`. True when something changed.
    pub fn tick(&mut self, now: DateTime<Utc>) -> bool {
        let mut changed = false;
        // Bounded: at most a lock and the three auto steps after it
        for _ in 0..4 {
            if let (Phase::Question, Some(d)) = (self.phase, self.deadline) {
                if now >= d {
                    self.lock(d);
                    changed = true;
                    continue;
                }
            }
            let Some(s) = self.spec.settings.auto_advance_s else { break };
            if !matches!(self.phase, Phase::Locked | Phase::Reveal | Phase::Leaderboard) {
                break;
            }
            let due = self.phase_since + Duration::seconds(i64::from(s));
            if now < due {
                break;
            }
            // A new question starts now, not when it was due: after the box sleeps, the game
            // carries on instead of racing through questions nobody saw.
            let at = if matches!(self.phase, Phase::Reveal | Phase::Leaderboard) { now } else { due };
            if self.advance(at).is_err() {
                break;
            }
            changed = true;
        }
        changed
    }

    pub fn next_deadline(&self) -> Option<DateTime<Utc>> {
        if let (Phase::Question, Some(d)) = (self.phase, self.deadline) {
            return Some(d);
        }
        match (self.phase, self.spec.settings.auto_advance_s) {
            (Phase::Locked | Phase::Reveal | Phase::Leaderboard, Some(s)) => Some(self.phase_since + Duration::seconds(i64::from(s))),
            _ => None,
        }
    }

    // ---- answers --------------------------------------------------------------------------

    /// A guest's answer to question `q` (the flat index the views show).
    pub fn answer(&mut self, id: &str, q: usize, value: &serde_json::Value, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        self.active(id)?;
        self.tick(now);
        if !self.started || q != self.current {
            return Err(ActionError::WrongPhase("that question isn't open".into()));
        }
        match self.phase {
            Phase::Question => {}
            Phase::Locked | Phase::Reveal | Phase::Leaderboard | Phase::Final => return Err(ActionError::TooLate),
            Phase::Lobby => return Err(ActionError::WrongPhase("that question isn't open".into())),
        }
        if let Some(d) = self.deadline {
            if now >= d {
                return Err(ActionError::TooLate);
            }
        }
        if self.record(q, id).is_some() {
            return Ok(ActionOutcome::Ignored("you've already answered this one".into()));
        }
        let question = self.question(q);
        let (value, verdict) = match question.kind {
            Some(Kind::Choice) => {
                let pos = value
                    .as_u64()
                    .map(|v| v as usize)
                    .filter(|&p| p < self.option_order[q].len())
                    .ok_or_else(|| ActionError::BadInput("pick one of the options".into()))?;
                let idx = self.option_order[q][pos];
                let v = if Some(idx) == self.right_option(q) { Verdict::Correct } else { Verdict::Wrong };
                (AnswerValue::Choice(idx), v)
            }
            Some(Kind::Text) => {
                let s = value.as_str().ok_or_else(|| ActionError::BadInput("type an answer".into()))?.trim();
                if s.is_empty() || s.chars().count() > MAX_GUEST_ANSWER {
                    return Err(ActionError::BadInput(format!("an answer is 1 to {MAX_GUEST_ANSWER} characters")));
                }
                let v = match matching::match_text(s, question.answer_text(), question.accepted()) {
                    TextMatch::Correct => Verdict::Correct,
                    TextMatch::NeedsJudge => Verdict::NeedsJudge,
                    TextMatch::Wrong => Verdict::Wrong,
                };
                (AnswerValue::Text(s.to_string()), v)
            }
            Some(Kind::Number) => {
                let n = parse_number(value).ok_or_else(|| ActionError::BadInput("type a number".into()))?;
                let right = question.answer_number().unwrap_or(f64::NAN);
                let v = if (n - right).abs() <= question.tolerance() + 1e-9 { Verdict::Correct } else { Verdict::Wrong };
                (AnswerValue::Number(n), v)
            }
            None => return Err(ActionError::BadInput("this question can't take answers".into())),
        };
        self.answers[q].push(AnswerRecord { player: id.to_string(), value, at: now, verdict });
        if self.spec.settings.lock_when_all_answered() && self.everyone_answered(q) {
            self.lock(now);
        }
        Ok(ActionOutcome::Applied)
    }

    fn everyone_answered(&self, q: usize) -> bool {
        let mut here = self.players.iter().filter(|p| !p.removed && p.connected).peekable();
        here.peek().is_some() && here.all(|p| self.record(q, &p.id).is_some())
    }

    /// A verdict on a player's answer. The judge hook may only settle a near miss; the host may
    /// overrule any answer.
    pub fn judge(&mut self, by_host: bool, q: usize, player: &str, verdict: JudgeVerdict) -> Result<ActionOutcome, ActionError> {
        if q >= self.qrefs.len() {
            return Err(ActionError::BadInput("no such question".into()));
        }
        let rec = self.answers[q]
            .iter_mut()
            .find(|a| a.player == player)
            .ok_or_else(|| ActionError::BadInput("that player didn't answer that question".into()))?;
        let pending = rec.verdict == Verdict::NeedsJudge;
        if !by_host && !pending {
            return Err(ActionError::NotAllowed("only a near miss waiting for a judge can be settled".into()));
        }
        let new = Verdict::Judged { verdict, by_host };
        if rec.verdict == new {
            return Ok(ActionOutcome::Ignored("already settled that way".into()));
        }
        rec.verdict = new;
        Ok(ActionOutcome::Applied)
    }

    pub fn adjust(&mut self, target: Target, points: i64) -> Result<ActionOutcome, ActionError> {
        if points == 0 || points.abs() > MAX_ADJUST {
            return Err(ActionError::BadInput(format!("give between -{MAX_ADJUST} and {MAX_ADJUST} points, not 0")));
        }
        match &target {
            Target::Player(id) => {
                self.active(id)?;
            }
            Target::Team(t) => {
                if !self.players.iter().any(|p| !p.removed && p.team.as_deref() == Some(t.as_str())) {
                    return Err(ActionError::BadInput("no such team".into()));
                }
            }
        }
        self.adjustments.push(Adjustment { target, points });
        Ok(ActionOutcome::Applied)
    }

    pub fn pending_judgements(&self) -> Vec<JudgeRequest> {
        let mut out = Vec::new();
        for (q, recs) in self.answers.iter().enumerate() {
            for r in recs {
                if r.verdict != Verdict::NeedsJudge || self.player(&r.player).is_none_or(|p| p.removed) {
                    continue;
                }
                let qq = self.question(q);
                if let AnswerValue::Text(guess) = &r.value {
                    out.push(JudgeRequest {
                        q,
                        player: r.player.clone(),
                        question: qq.text.clone(),
                        answer: qq.answer_text().to_string(),
                        accept: qq.accepted().to_vec(),
                        guess: guess.clone(),
                    });
                }
            }
        }
        out
    }

    // ---- scores ---------------------------------------------------------------------------

    /// Points one answer earns.
    pub fn points_for(&self, q: usize, rec: &AnswerRecord) -> i64 {
        let p = i64::from(self.question(q).points());
        match rec.verdict {
            Verdict::Correct | Verdict::Judged { verdict: JudgeVerdict::Yes, .. } => p,
            Verdict::Judged { verdict: JudgeVerdict::Partial, .. } => p / 2,
            Verdict::Wrong | Verdict::NeedsJudge | Verdict::Judged { verdict: JudgeVerdict::No, .. } => 0,
        }
    }

    /// A player's score over revealed questions, plus the host's adjustments.
    pub fn score(&self, id: &str) -> i64 {
        let revealed = self.revealed_count();
        let answered: i64 = (0..revealed).filter_map(|q| self.record(q, id).map(|r| self.points_for(q, r))).sum();
        let adjusted: i64 =
            self.adjustments.iter().filter(|a| matches!(&a.target, Target::Player(p) if p == id)).map(|a| a.points).sum();
        answered + adjusted
    }

    /// Every player's score in one pass (same rule as [`Self::score`]).
    fn all_scores(&self) -> std::collections::HashMap<&str, i64> {
        let mut out: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
        for q in 0..self.revealed_count().min(self.answers.len()) {
            for r in &self.answers[q] {
                *out.entry(r.player.as_str()).or_default() += self.points_for(q, r);
            }
        }
        for a in &self.adjustments {
            if let Target::Player(p) = &a.target {
                *out.entry(p.as_str()).or_default() += a.points;
            }
        }
        out
    }

    /// Players still in the game, best first (ties keep join order), with shared ranks.
    pub fn leaderboard(&self) -> Vec<ScoreLine> {
        let scores = self.all_scores();
        let mut rows: Vec<ScoreLine> = self
            .players
            .iter()
            .filter(|p| !p.removed)
            .map(|p| ScoreLine {
                player: p.id.clone(),
                name: p.name.clone(),
                team: p.team.clone(),
                score: scores.get(p.id.as_str()).copied().unwrap_or(0),
                rank: 0,
                tied: false,
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.score));
        let scores: Vec<i64> = rows.iter().map(|r| r.score).collect();
        for (i, r) in rows.iter_mut().enumerate() {
            r.rank = scores.iter().position(|s| *s == r.score).unwrap_or(i) + 1;
            r.tied = scores.iter().filter(|s| **s == r.score).count() > 1;
        }
        rows
    }

    /// Teams, best first: members' points plus the team's own adjustments.
    pub fn team_board(&self) -> Vec<TeamLine> {
        if self.spec.teams.mode != TeamMode::Teams {
            return Vec::new();
        }
        let board = self.leaderboard();
        let mut teams: Vec<TeamLine> = Vec::new();
        for row in &board {
            let Some(t) = &row.team else { continue };
            match teams.iter_mut().find(|x| &x.team == t) {
                Some(x) => {
                    x.members += 1;
                    x.score += row.score;
                }
                None => teams.push(TeamLine { team: t.clone(), members: 1, score: row.score, rank: 0, tied: false }),
            }
        }
        for a in &self.adjustments {
            if let Target::Team(t) = &a.target {
                if let Some(x) = teams.iter_mut().find(|x| &x.team == t) {
                    x.score += a.points;
                }
            }
        }
        teams.sort_by_key(|t| std::cmp::Reverse(t.score));
        let scores: Vec<i64> = teams.iter().map(|t| t.score).collect();
        for (i, t) in teams.iter_mut().enumerate() {
            t.rank = scores.iter().position(|s| *s == t.score).unwrap_or(i) + 1;
            t.tied = scores.iter().filter(|s| **s == t.score).count() > 1;
        }
        teams
    }
}

/// A number from a JSON number, or from text like "1066", "3,5" or "1 000".
pub fn parse_number(v: &serde_json::Value) -> Option<f64> {
    let n = match v {
        serde_json::Value::Number(n) => n.as_f64()?,
        serde_json::Value::String(s) => {
            let s: String = s.trim().chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
            if s.is_empty() || s.len() > 40 {
                return None;
            }
            let s = if s.contains('.') { s.replace(',', "") } else if s.matches(',').count() == 1 { s.replace(',', ".") } else { s.replace(',', "") };
            s.parse::<f64>().ok()?
        }
        _ => return None,
    };
    n.is_finite().then_some(n)
}
