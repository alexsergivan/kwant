//! The dry run: N pretend players with a seeded RNG play every question (some right, some wrong,
//! some near misses, some late, some silent), with K of them dropping and rejoining in the middle
//! of a question, and the game saved and restored mid-question as if the box restarted.
//!
//! The run keeps its own ledger of what each pretend player meant to do and what that should
//! score, and compares it with the engine's scores after every reveal and at the end. It also
//! reads every player's view before each reveal to check nobody is shown the answer or another
//! player's answer. Anything that doesn't add up is a problem in the report.

use super::engine::{JudgeVerdict, Phase, QuizGame, ScoreLine, TeamLine, Verdict};
use super::matching::{self, normalise, TextMatch};
use super::spec::{Kind, QuizSpec, TeamMode};
use crate::kits::rng::SeededRng;
use crate::kits::{ActionError, ActionOutcome, Actor, Viewer};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;

/// Weights for what a pretend player does with a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerMix {
    pub right: u32,
    pub wrong: u32,
    pub near: u32,
    pub late: u32,
    pub silent: u32,
}

impl Default for AnswerMix {
    fn default() -> Self {
        Self { right: 55, wrong: 20, near: 10, late: 8, silent: 7 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunOptions {
    pub players: usize,
    /// How many players drop in the middle of a question and rejoin.
    pub drop: usize,
    pub seed: u64,
    pub mix: AnswerMix,
    /// Save and restore the game mid-question, as if the box restarted.
    pub restart_mid_game: bool,
    /// Read every player's view before and after each reveal (the leak check).
    pub check_views: bool,
}

impl Default for DryRunOptions {
    fn default() -> Self {
        Self { players: 30, drop: 0, seed: 1, mix: AnswerMix::default(), restart_mid_game: true, check_views: true }
    }
}

/// What a pretend player meant to do with one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "verdict")]
pub enum Intent {
    Right,
    Wrong,
    /// A near miss, and the verdict the pretend judge gives it.
    Near(JudgeVerdict),
    /// Answers (rightly) after time is up: must be refused and score nothing.
    Late,
    Silent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerPlan {
    pub id: String,
    pub name: String,
    pub team: Option<String>,
    pub joined: bool,
    pub dropped: bool,
    pub intents: Vec<Intent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionReport {
    pub q: usize,
    pub at: String,
    pub kind: &'static str,
    pub answers: usize,
    pub right: usize,
    pub wrong: usize,
    pub near_misses: usize,
    pub late_refused: usize,
    pub silent: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DryRunReport {
    pub ok: bool,
    /// One line for the Pocket card ("Dry run: 30 players, 40 questions, no problems").
    pub summary: String,
    pub players: usize,
    pub questions: usize,
    pub seed: u64,
    pub drop_question: Option<usize>,
    pub dropped: Vec<String>,
    pub restarted_at: Option<usize>,
    pub per_question: Vec<QuestionReport>,
    pub final_scores: Vec<ScoreLine>,
    pub teams: Vec<TeamLine>,
    pub winners: Vec<String>,
    pub problems: Vec<String>,
    /// Wall-clock time of the run; the run is single-threaded, so CPU time is at most this.
    pub elapsed_ms: f64,
    pub plan: Vec<PlayerPlan>,
}

/// The fixed start of a dry run's pretend clock (no real clock is read for game logic).
fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap_or_default()
}

/// Points an intent should earn, worked out from the spec alone (not from the engine).
pub fn expected_points(spec_points: u32, intent: Intent) -> i64 {
    let p = i64::from(spec_points);
    match intent {
        Intent::Right | Intent::Near(JudgeVerdict::Yes) => p,
        Intent::Near(JudgeVerdict::Partial) => p / 2,
        Intent::Near(JudgeVerdict::No) | Intent::Wrong | Intent::Late | Intent::Silent => 0,
    }
}

fn pick_intent(rng: &mut SeededRng, mix: &AnswerMix) -> Intent {
    let total = u64::from(mix.right + mix.wrong + mix.near + mix.late + mix.silent).max(1);
    let mut r = rng.below(total);
    for (w, i) in [
        (mix.right, Intent::Right),
        (mix.wrong, Intent::Wrong),
        (mix.near, Intent::Near(JudgeVerdict::Yes)),
        (mix.late, Intent::Late),
        (mix.silent, Intent::Silent),
    ] {
        if r < u64::from(w) {
            return i;
        }
        r -= u64::from(w);
    }
    Intent::Silent
}

fn wrong_token(p: usize, q: usize) -> String {
    format!("wrongp{p}q{q}")
}

/// A near miss for a text answer: one letter in the middle changed. None when the answer is too
/// short for a near miss or the change happens to match exactly.
fn near_miss(answer: &str, accept: &[String]) -> Option<String> {
    let norm = normalise(answer);
    let mut chars: Vec<char> = norm.chars().collect();
    let letters: Vec<usize> = chars.iter().enumerate().filter(|(_, c)| c.is_alphabetic()).map(|(i, _)| i).collect();
    if letters.len() < 4 {
        return None;
    }
    let i = letters[letters.len() / 2];
    chars[i] = if chars[i] == 'x' { 'q' } else { 'x' };
    let guess: String = chars.into_iter().collect();
    (matching::match_text(&guess, answer, accept) == TextMatch::NeedsJudge).then_some(guess)
}

struct Sim<'a> {
    spec: &'a QuizSpec,
    game: QuizGame,
    problems: Vec<String>,
    plan: Vec<PlayerPlan>,
    expected: Vec<i64>,
}

enum Ev {
    Answer(usize, Value),
    Drop(usize),
    Rejoin(usize),
    Restart,
}

impl Sim<'_> {
    fn at(&self, q: usize) -> String {
        let r = self.game.qref(q);
        format!("round {}, question {}", r.round + 1, r.index + 1)
    }

    fn problem(&mut self, msg: String) {
        if self.problems.len() < 200 {
            self.problems.push(msg);
        }
    }

    /// The value a pretend player sends for an intent.
    fn value_for(&self, rng: &mut SeededRng, p: usize, q: usize, intent: Intent) -> Option<Value> {
        let qq = self.game.question(q);
        let order = self.game.option_order(q);
        let right_pos = self.game.right_option(q).and_then(|i| order.iter().position(|&o| o == i));
        match (qq.kind, intent) {
            (_, Intent::Silent) => None,
            (Some(Kind::Choice | Kind::Number), Intent::Near(_)) => None,
            (Some(Kind::Choice), Intent::Right | Intent::Late) => right_pos.map(|x| json!(x)),
            (Some(Kind::Choice), _) => {
                let rp = right_pos.unwrap_or(0);
                let others: Vec<usize> = (0..order.len()).filter(|&x| x != rp).collect();
                others.get(rng.below(others.len() as u64) as usize).map(|x| json!(x))
            }
            (Some(Kind::Text), Intent::Right | Intent::Late) => {
                let mut spellings = vec![qq.answer_text().to_string()];
                spellings.extend(qq.accepted().iter().cloned());
                let s = spellings[rng.below(spellings.len() as u64) as usize].clone();
                Some(json!(match rng.below(3) {
                    0 => s.to_uppercase(),
                    1 => format!("  {}  ", s.to_lowercase()),
                    _ => s,
                }))
            }
            (Some(Kind::Text), Intent::Near(_)) => near_miss(qq.answer_text(), qq.accepted()).map(|s| json!(s)),
            (Some(Kind::Text), _) => Some(json!(wrong_token(p, q))),
            (Some(Kind::Number), Intent::Right | Intent::Late) => {
                let a = qq.answer_number().unwrap_or_default();
                Some(if rng.below(2) == 0 { json!(a + qq.tolerance() / 2.0) } else { json!(a.to_string()) })
            }
            (Some(Kind::Number), _) => {
                let a = qq.answer_number().unwrap_or_default();
                Some(json!(a + qq.tolerance() + 1.0 + rng.below(100) as f64))
            }
            (None, _) => None,
        }
    }

    fn guest_view(&self, p: usize, now: DateTime<Utc>) -> String {
        serde_json::to_string(&self.game.view(&Viewer::Guest(self.plan[p].id.clone()), now)).unwrap_or_default()
    }

    /// Before a reveal: no guest and not the big screen may hold the answer or anyone's answer.
    fn check_hidden(&mut self, q: usize, now: DateTime<Utc>) {
        let qq = self.game.question(q);
        let answer = normalise(qq.answer_text());
        // Words every view may show anyway: if the answer hides in them, the text check is skipped
        let r = self.game.qref(q);
        let round = &self.spec.rounds[r.round];
        let mut shown = vec![qq.text.clone(), self.spec.title.clone(), round.title.clone()];
        shown.extend(round.intro.clone());
        shown.extend(self.spec.lines.intro.clone());
        shown.extend(qq.options().iter().cloned());
        shown.extend(self.plan.iter().map(|p| format!("{} {}", p.name, p.team.clone().unwrap_or_default())));
        let in_question = shown.iter().any(|s| normalise(s).contains(&answer));
        let check_text = qq.kind == Some(Kind::Text) && answer.chars().count() >= 4 && !in_question;
        let src = qq.source.as_ref().map(|s| s.quote.clone()).unwrap_or_default();
        let tokens: Vec<(usize, String)> = (0..self.plan.len()).map(|p| (p, wrong_token(p, q))).collect();
        let mut found = Vec::new();
        for p in 0..self.plan.len() {
            if !self.plan[p].joined {
                continue;
            }
            let v = self.guest_view(p, now);
            let lower = v.to_lowercase();
            if lower.contains("\"reveal\"") || lower.contains("\"result\"") {
                found.push(format!("{}: {} was shown the reveal before it happened", self.at(q), self.plan[p].name));
            }
            if !src.is_empty() && v.contains(&src) {
                found.push(format!("{}: {} was shown the source before the reveal", self.at(q), self.plan[p].name));
            }
            let own_holds_answer = matches!(self.plan[p].intents[q], Intent::Right | Intent::Near(_));
            if check_text && !own_holds_answer && normalise(&v).contains(&answer) {
                found.push(format!("{}: {} was shown the answer before the reveal", self.at(q), self.plan[p].name));
            }
            if let Some((o, _)) = tokens.iter().find(|(o, t)| *o != p && v.contains(t.as_str())) {
                found.push(format!("{}: {} was shown {}'s answer", self.at(q), self.plan[p].name, self.plan[*o].name));
            }
        }
        let screen = serde_json::to_string(&self.game.view(&Viewer::BigScreen, now)).unwrap_or_default();
        if screen.contains("\"reveal\"") || (check_text && normalise(&screen).contains(&answer)) {
            found.push(format!("{}: the big screen showed the answer before the reveal", self.at(q)));
        }
        if tokens.iter().any(|(_, t)| screen.contains(t.as_str())) {
            found.push(format!("{}: the big screen showed a player's answer", self.at(q)));
        }
        for f in found {
            self.problem(f);
        }
    }

    /// After a reveal: every guest sees it, sees their own result, and their score matches the ledger.
    fn check_revealed(&mut self, q: usize, now: DateTime<Utc>) {
        let tokens: Vec<String> = (0..self.plan.len()).map(|p| wrong_token(p, q)).collect();
        let mut found = Vec::new();
        for p in 0..self.plan.len() {
            if !self.plan[p].joined {
                continue;
            }
            let view = self.game.view(&Viewer::Guest(self.plan[p].id.clone()), now);
            if view.reveal.is_none() || view.me.as_ref().and_then(|m| m.result.as_ref()).is_none() {
                found.push(format!("{}: {} didn't see the reveal", self.at(q), self.plan[p].name));
            }
            let score = view.me.as_ref().map(|m| m.score).unwrap_or_default();
            if score != self.expected[p] {
                found.push(format!(
                    "{}: {} has {} points after the reveal; the ledger says {}",
                    self.at(q),
                    self.plan[p].name,
                    score,
                    self.expected[p]
                ));
            }
            let v = serde_json::to_string(&view).unwrap_or_default();
            if tokens.iter().enumerate().any(|(o, t)| o != p && v.contains(t.as_str())) {
                found.push(format!("{}: {} was shown another player's answer", self.at(q), self.plan[p].name));
            }
        }
        for f in found {
            self.problem(f);
        }
    }

    fn apply(&mut self, q: usize, ev: &Ev, now: DateTime<Utc>, late_refused: &mut usize) {
        self.game.tick(now);
        match ev {
            Ev::Answer(p, value) => {
                let p = *p;
                let id = self.plan[p].id.clone();
                let intent = self.plan[p].intents[q];
                let res = self.game.act(&Actor::Guest(id.clone()), "answer", &json!({"q": q, "value": value}), now);
                match (intent, res) {
                    (Intent::Late, Err(ActionError::TooLate)) => *late_refused += 1,
                    (Intent::Late, other) => {
                        self.problem(format!("{}: {}'s late answer wasn't refused ({other:?})", self.at(q), self.plan[p].name))
                    }
                    (_, Ok(ActionOutcome::Applied)) => {
                        let got = self.game.record(q, &id).map(|r| r.verdict);
                        let want = match intent {
                            Intent::Right => Verdict::Correct,
                            Intent::Near(_) => Verdict::NeedsJudge,
                            _ => Verdict::Wrong,
                        };
                        if got != Some(want) {
                            self.problem(format!("{}: {}'s answer {value} was marked {got:?}, not {want:?}", self.at(q), self.plan[p].name));
                        }
                    }
                    (_, other) => {
                        self.problem(format!("{}: {}'s answer {value} was refused ({other:?})", self.at(q), self.plan[p].name))
                    }
                }
            }
            Ev::Drop(p) => {
                let id = self.plan[*p].id.clone();
                self.game.set_connected(&id, false);
            }
            Ev::Rejoin(p) => self.rejoin(*p, q, now),
            Ev::Restart => self.restart(q),
        }
    }

    fn rejoin(&mut self, p: usize, q: usize, now: DateTime<Utc>) {
        let (id, name) = (self.plan[p].id.clone(), self.plan[p].name.clone());
        let before = self.game.score(&id);
        match self.game.join(&id, &name, now) {
            Ok(ActionOutcome::Ignored(_)) => {}
            other => self.problem(format!("{}: {name} couldn't rejoin ({other:?})", self.at(q))),
        }
        if self.game.score(&id) != before {
            self.problem(format!("{}: {name}'s score changed on rejoining", self.at(q)));
        }
    }

    fn restart(&mut self, q: usize) {
        let saved = self.game.save();
        match QuizGame::restore(self.spec.clone(), &saved) {
            Ok(g) => {
                if g.save() != saved {
                    self.problem(format!("{}: the game came back different after a restart", self.at(q)));
                }
                self.game = g;
            }
            Err(e) => self.problem(format!("{}: the game couldn't be restored after a restart: {e}", self.at(q))),
        }
    }
}

/// Plays the spec (already validated) with pretend players.
pub fn run(spec: &QuizSpec, opts: &DryRunOptions) -> DryRunReport {
    let started = Instant::now();
    let mut rng = SeededRng::new(opts.seed);
    let t0 = epoch();
    let game = QuizGame::new(spec.clone(), opts.seed, t0);
    let total = game.total();
    let n = opts.players;

    let mut sim = Sim { spec, game, problems: Vec::new(), plan: Vec::new(), expected: vec![0; n] };

    // Teams: fill each team up to its size, in order.
    let team_size = spec.teams.size.unwrap_or(1).max(1) as usize;
    let mut seats_note = None;
    for p in 0..n {
        let team = match spec.teams.mode {
            TeamMode::Solo => None,
            TeamMode::Teams => {
                let t = p / team_size;
                match &spec.teams.names {
                    Some(names) if t < names.len() => Some(names[t].trim().to_string()),
                    Some(names) => {
                        seats_note = Some(format!(
                            "teams: {} named teams of {} seat {} players; the dry run had {n}",
                            names.len(),
                            team_size,
                            names.len() * team_size
                        ));
                        None
                    }
                    None => Some(format!("Team {}", t + 1)),
                }
            }
        };
        let intents = (0..total)
            .map(|_| match pick_intent(&mut rng, &opts.mix) {
                Intent::Near(_) => Intent::Near([JudgeVerdict::Yes, JudgeVerdict::No, JudgeVerdict::Partial][rng.below(3) as usize]),
                i => i,
            })
            .collect();
        sim.plan.push(PlayerPlan { id: format!("p{p}"), name: format!("Player {}", p + 1), team, joined: false, dropped: false, intents });
    }
    if let Some(note) = seats_note {
        sim.problem(note);
    }

    // Who drops, and on which question.
    let drop = opts.drop.min(n);
    let drop_q = (drop > 0 && total > 0).then_some(total / 2);
    let mut idx: Vec<usize> = (0..n).collect();
    rng.shuffle(&mut idx);
    let dropped: Vec<usize> = idx.into_iter().take(drop).collect();
    let (group_a, group_b) = dropped.split_at(drop / 2 + drop % 2);
    if let Some(dq) = drop_q {
        for &p in &dropped {
            sim.plan[p].dropped = true;
            if matches!(sim.plan[p].intents[dq], Intent::Late | Intent::Silent) {
                sim.plan[p].intents[dq] = Intent::Right;
            }
        }
    }
    let restart_q = (opts.restart_mid_game && total > 0).then_some(total / 2);

    // Lobby: everyone joins and finds a team.
    let mut now = t0;
    for p in 0..n {
        now += Duration::milliseconds(100);
        let (id, name) = (sim.plan[p].id.clone(), sim.plan[p].name.clone());
        match sim.game.join(&id, &name, now) {
            Ok(ActionOutcome::Applied) => sim.plan[p].joined = true,
            other => sim.problem(format!("lobby: {name} couldn't join ({other:?})")),
        }
        if let (true, Some(team)) = (sim.plan[p].joined, sim.plan[p].team.clone()) {
            if let Err(e) = sim.game.act(&Actor::Guest(id), "team", &json!({"name": team}), now) {
                sim.problem(format!("lobby: {name} couldn't join {team}: {e}"));
            }
        }
    }
    for plan in sim.plan.iter_mut().take(n) {
        if !plan.joined {
            plan.intents.iter_mut().for_each(|i| *i = Intent::Silent);
        }
    }
    now += Duration::seconds(5);
    if let Err(e) = sim.game.act(&Actor::Host, "start", &json!({}), now) {
        sim.problem(format!("lobby: the quiz wouldn't start: {e}"));
    }

    let mut per_question = Vec::with_capacity(total);
    for q in 0..total {
        if sim.game.phase() != Phase::Question || sim.game.current() != q {
            sim.problem(format!("{}: the question didn't open (phase {:?})", sim.at(q), sim.game.phase()));
            break;
        }
        let qstart = now;
        let limit_ms = i64::from(sim.game.question(q).time_limit_s()) * 1000;
        let mut events: Vec<(i64, u8, Ev)> = Vec::new();
        let mut silent = 0;
        for p in 0..n {
            if !sim.plan[p].joined {
                continue;
            }
            let mut intent = sim.plan[p].intents[q];
            let mut value = sim.value_for(&mut rng, p, q, intent);
            if matches!(intent, Intent::Near(_)) && value.is_none() {
                // Too short (or not text) for a near miss: a plain wrong answer instead
                intent = Intent::Wrong;
                sim.plan[p].intents[q] = intent;
                value = sim.value_for(&mut rng, p, q, intent);
            }
            let Some(value) = value else {
                silent += 1;
                continue;
            };
            let in_a = drop_q == Some(q) && group_a.contains(&p);
            let in_b = drop_q == Some(q) && group_b.contains(&p);
            if in_a {
                events.push((200, 0, Ev::Answer(p, value)));
                events.push((500, 1, Ev::Drop(p)));
            } else if in_b {
                events.push((500, 1, Ev::Drop(p)));
                events.push((2000, 2, Ev::Rejoin(p)));
                events.push((3000, 3, Ev::Answer(p, value)));
            } else if intent == Intent::Late {
                events.push((limit_ms + 1000, 5, Ev::Answer(p, value)));
            } else {
                let t = rng.between(4000, (limit_ms - 1000).max(4000) as u64) as i64;
                events.push((t, 4, Ev::Answer(p, value)));
            }
        }
        if restart_q == Some(q) {
            events.push((1000, 2, Ev::Restart));
        }
        events.sort_by_key(|(t, o, _)| (*t, *o));

        let mut late_refused = 0;
        let mut checked_hidden = false;
        for (t, _, ev) in &events {
            let at = qstart + Duration::milliseconds(*t);
            if opts.check_views && !checked_hidden && *t >= limit_ms {
                sim.check_hidden(q, at);
                checked_hidden = true;
            }
            sim.apply(q, ev, at, &mut late_refused);
            now = now.max(at);
        }
        let deadline = qstart + Duration::milliseconds(limit_ms);
        now = now.max(deadline);
        sim.game.tick(now);
        if sim.game.phase() != Phase::Locked {
            sim.problem(format!("{}: the timer ended but the question didn't lock (phase {:?})", sim.at(q), sim.game.phase()));
        }
        if opts.check_views && !checked_hidden {
            sim.check_hidden(q, now);
        }

        // The pretend judge settles near misses with the verdict each player was planned to get.
        let recs = sim.game.answers_for(q).to_vec();
        let near_misses = recs.iter().filter(|r| r.verdict == Verdict::NeedsJudge).count();
        for r in &recs {
            let Some(p) = sim.plan.iter().position(|pl| pl.id == r.player) else { continue };
            if let (Verdict::NeedsJudge, Intent::Near(v)) = (r.verdict, sim.plan[p].intents[q]) {
                let word = match v {
                    JudgeVerdict::Yes => "yes",
                    JudgeVerdict::No => "no",
                    JudgeVerdict::Partial => "partial",
                };
                if let Err(e) = sim.game.act(&Actor::Judge, "judge", &json!({"q": q, "player": r.player, "verdict": word}), now) {
                    sim.problem(format!("{}: the judge couldn't settle {}'s near miss: {e}", sim.at(q), sim.plan[p].name));
                }
            }
        }

        // The ledger: what each player should have now.
        let points = sim.game.question(q).points();
        for p in 0..n {
            sim.expected[p] += expected_points(points, sim.plan[p].intents[q]);
        }

        now += Duration::seconds(2);
        if let Err(e) = sim.game.act(&Actor::Host, "next", &json!({}), now) {
            sim.problem(format!("{}: Next wouldn't reveal: {e}", sim.at(q)));
        }
        if drop_q == Some(q) {
            for &p in group_a {
                sim.rejoin(p, q, now);
            }
        }
        if opts.check_views {
            sim.check_revealed(q, now);
        }

        let recs = sim.game.answers_for(q);
        per_question.push(QuestionReport {
            q,
            at: sim.at(q),
            kind: sim.game.question(q).kind.map(Kind::as_str).unwrap_or("text"),
            answers: recs.len(),
            right: recs.iter().filter(|r| r.verdict == Verdict::Correct).count(),
            wrong: recs.iter().filter(|r| r.verdict == Verdict::Wrong).count(),
            near_misses,
            late_refused,
            silent,
        });

        // On to the next question (through the leaderboard when one is due).
        for _ in 0..2 {
            now += Duration::seconds(3);
            if let Err(e) = sim.game.act(&Actor::Host, "next", &json!({}), now) {
                sim.problem(format!("{}: Next didn't move on: {e}", sim.at(q)));
                break;
            }
            if matches!(sim.game.phase(), Phase::Question | Phase::Final) {
                break;
            }
        }
    }

    if total > 0 && sim.game.phase() != Phase::Final {
        sim.problem(format!("the quiz didn't reach the final screen (phase {:?})", sim.game.phase()));
    }
    let final_scores = sim.game.leaderboard();
    for p in 0..n {
        if !sim.plan[p].joined {
            continue;
        }
        let got = final_scores.iter().find(|r| r.player == sim.plan[p].id).map(|r| r.score);
        if got != Some(sim.expected[p]) {
            let name = sim.plan[p].name.clone();
            let want = sim.expected[p];
            sim.problem(format!("final: {name} has {got:?} points; the ledger says {want}"));
        }
    }
    let teams = sim.game.team_board();
    for t in &teams {
        let want: i64 = (0..n).filter(|&p| sim.plan[p].joined && sim.plan[p].team.as_deref() == Some(t.team.as_str())).map(|p| sim.expected[p]).sum();
        if t.score != want {
            sim.problem(format!("final: team {} has {} points; the ledger says {want}", t.team, t.score));
        }
    }
    let winners = if teams.is_empty() {
        final_scores.iter().filter(|r| r.rank == 1).map(|r| r.name.clone()).collect()
    } else {
        teams.iter().filter(|t| t.rank == 1).map(|t| t.team.clone()).collect()
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let ok = sim.problems.is_empty();
    let summary = if ok {
        format!("Dry run: {n} players, {total} questions, no problems")
    } else {
        format!("Dry run: {n} players, {total} questions, {} problems", sim.problems.len())
    };
    DryRunReport {
        ok,
        summary,
        players: n,
        questions: total,
        seed: opts.seed,
        drop_question: drop_q,
        dropped: dropped.iter().map(|&p| sim.plan[p].id.clone()).collect(),
        restarted_at: restart_q,
        per_question,
        final_scores,
        teams,
        winners,
        problems: sim.problems,
        elapsed_ms,
        plan: sim.plan,
    }
}
