//! What each viewer may see (Pocket API v1 view shapes; written up in pa2/impl.md).
//!
//! - A guest sees the open question, their own answer and score, and the leaderboard. Never the
//!   answer, the accepted spellings or the source before the reveal, never anyone else's answer,
//!   and never a score that moved because of an answer not yet revealed.
//! - The big screen is public: the question, the timer, who has answered (names only), and after
//!   the reveal the answer, the source and how many picked each option.
//! - The host remote sees everything, including the answer ahead and the near misses to settle.

use super::engine::{AnswerValue, JudgeRequest, Phase, QuizGame, ScoreLine, TeamLine, Verdict};
use super::spec::{Kind, TeamMode};
use crate::kits::Viewer;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

/// How many rows the big screen's leaderboard shows.
pub const BIG_SCREEN_ROWS: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct QuizView {
    pub kit: &'static str,
    pub viewer: &'static str,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub phase: Phase,
    pub teams: bool,
    /// The host's intro (lobby) or final line (final).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<RoundView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<QuestionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<TimeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered: Option<AnsweredView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reveal: Option<RevealView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaderboard: Option<Vec<BoardRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_board: Option<Vec<TeamRow>>,
    /// Lobby: who is in (names and teams).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lobby: Option<Vec<LobbyRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub me: Option<MeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<HostView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundView {
    pub number: usize,
    pub of: usize,
    pub title: String,
    /// The round's intro line, on its first question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuestionView {
    /// Flat index, 0-based: the `q` an answer names.
    pub q: usize,
    pub number: usize,
    pub of: usize,
    pub kind: &'static str,
    pub text: String,
    /// Choice: the options in this game's order; an answer names a position in this list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<PictureView>,
    pub points: u32,
    pub time_limit_s: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PictureView {
    pub url: String,
    pub licence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeView {
    /// RFC 3339; absent while paused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    pub remaining_ms: i64,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnsweredView {
    pub count: usize,
    pub of: usize,
    /// Big screen and host: who has answered (names, never what).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevealView {
    pub q: usize,
    /// The answer as people read it ("1066 (± 2)" for a number with a tolerance).
    pub answer: String,
    /// Choice: the right option's position in this game's order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub source_url: String,
    pub source_quote: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Big screen and host: how many picked each option (in this game's order), no names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picks: Option<Vec<usize>>,
    /// Big screen and host: how many got it right.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardRow {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub score: i64,
    pub rank: usize,
    pub tied: bool,
    /// Guest view only: this row is the viewer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub me: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamRow {
    pub team: String,
    pub members: usize,
    pub score: i64,
    pub rank: usize,
    pub tied: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LobbyRow {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeView {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub removed: bool,
    pub score: i64,
    pub rank: usize,
    pub of: usize,
    /// Whether I've answered the current question.
    pub answered: bool,
    /// My own answer to the current question, as I sent it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_answer: Option<String>,
    /// After the reveal only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultView {
    /// correct | wrong | partial | waiting_for_judge | no_answer
    pub verdict: &'static str,
    pub points: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    /// The current question's answer, ahead of the reveal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_now: Option<RevealView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
    /// Every answer to the current question.
    pub answers: Vec<HostAnswer>,
    /// Near misses waiting for a verdict, from any question.
    pub pending: Vec<JudgeRequest>,
    pub players: Vec<HostPlayer>,
    /// Actions that make sense right now.
    pub can: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostAnswer {
    pub player: String,
    pub name: String,
    pub answer: String,
    pub verdict: &'static str,
    pub points: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostPlayer {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub connected: bool,
    pub removed: bool,
    pub score: i64,
}

fn verdict_word(v: Verdict) -> &'static str {
    use super::engine::JudgeVerdict as J;
    match v {
        Verdict::Correct | Verdict::Judged { verdict: J::Yes, .. } => "correct",
        Verdict::Wrong | Verdict::Judged { verdict: J::No, .. } => "wrong",
        Verdict::Judged { verdict: J::Partial, .. } => "partial",
        Verdict::NeedsJudge => "waiting_for_judge",
    }
}

fn fmt_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

impl QuizGame {
    /// The text of an answer as people read it.
    pub(crate) fn answer_display(&self, q: usize, v: &AnswerValue) -> String {
        match v {
            AnswerValue::Choice(i) => self.question(q).options().get(*i).cloned().unwrap_or_default(),
            AnswerValue::Text(s) => s.clone(),
            AnswerValue::Number(n) => fmt_number(*n),
        }
    }

    fn reveal_view(&self, q: usize, public_counts: bool) -> RevealView {
        let qq = self.question(q);
        let answer = match qq.kind {
            Some(Kind::Number) => {
                let n = fmt_number(qq.answer_number().unwrap_or_default());
                if qq.tolerance() > 0.0 {
                    format!("{n} (± {})", fmt_number(qq.tolerance()))
                } else {
                    n
                }
            }
            _ => qq.answer_text().trim().to_string(),
        };
        let right_idx = self.right_option(q);
        let order = self.option_order(q);
        let position = if qq.kind == Some(Kind::Choice) { right_idx.and_then(|i| order.iter().position(|&o| o == i)) } else { None };
        let line = qq.reveal_line.clone().or_else(|| {
            let l = &self.spec.lines.reveal;
            (!l.is_empty()).then(|| l[q % l.len()].clone())
        });
        let src = qq.source.clone().unwrap_or_default();
        let (picks, right) = if public_counts {
            let recs: Vec<_> = self.answers_for(q).iter().filter(|r| self.player(&r.player).is_some_and(|p| !p.removed)).collect();
            let picks = (qq.kind == Some(Kind::Choice)).then(|| {
                order.iter().map(|idx| recs.iter().filter(|r| r.value == AnswerValue::Choice(*idx)).count()).collect()
            });
            let right = recs.iter().filter(|r| self.points_for(q, r) > 0).count();
            (picks, Some(right))
        } else {
            (None, None)
        };
        RevealView { q, answer, position, source_url: src.url, source_quote: src.quote, line, picks, right }
    }

    fn question_view(&self, q: usize) -> QuestionView {
        let qq = self.question(q);
        let options = (qq.kind == Some(Kind::Choice))
            .then(|| self.option_order(q).iter().map(|&i| qq.options()[i].trim().to_string()).collect());
        let picture = qq.picture.as_ref().map(|p| PictureView {
            url: p.url.trim().to_string(),
            licence: p.licence.clone().unwrap_or_default(),
            credit: p.credit.clone(),
            alt: p.alt.clone(),
        });
        QuestionView {
            q,
            number: q + 1,
            of: self.total(),
            kind: qq.kind.map(Kind::as_str).unwrap_or("text"),
            text: qq.text.trim().to_string(),
            options,
            picture,
            points: qq.points(),
            time_limit_s: qq.time_limit_s(),
        }
    }

    fn board_rows(rows: &[ScoreLine], me: Option<&str>, limit: usize) -> Vec<BoardRow> {
        rows.iter()
            .enumerate()
            .filter(|(i, r)| *i < limit || me == Some(r.player.as_str()))
            .map(|(_, r)| BoardRow {
                name: r.name.clone(),
                team: r.team.clone(),
                score: r.score,
                rank: r.rank,
                tied: r.tied,
                me: me == Some(r.player.as_str()),
            })
            .collect()
    }

    fn team_rows(rows: &[TeamLine]) -> Vec<TeamRow> {
        rows.iter().map(|t| TeamRow { team: t.team.clone(), members: t.members, score: t.score, rank: t.rank, tied: t.tied }).collect()
    }

    /// Actions the host can take in this phase.
    fn host_can(&self) -> Vec<&'static str> {
        let mut can = match self.phase {
            Phase::Lobby => vec!["start"],
            Phase::Question if self.deadline.is_none() => vec!["next", "resume"],
            Phase::Question => vec!["next", "pause"],
            Phase::Locked | Phase::Reveal | Phase::Leaderboard => vec!["next"],
            Phase::Final => vec![],
        };
        if self.phase != Phase::Final {
            can.extend(["adjust", "remove", "end"]);
        }
        if self.started {
            can.push("judge");
        }
        can
    }

    pub fn view(&self, viewer: &Viewer, now: DateTime<Utc>) -> QuizView {
        let (who, me_id) = match viewer {
            Viewer::Guest(id) => ("guest", Some(id.as_str())),
            Viewer::Host => ("host", None),
            Viewer::BigScreen => ("big_screen", None),
        };
        let is_host = matches!(viewer, Viewer::Host);
        let is_screen = matches!(viewer, Viewer::BigScreen);
        let in_play = self.started && self.phase != Phase::Lobby;
        let q = self.current;
        let qref = in_play.then(|| self.qref(q));

        let line = match self.phase {
            Phase::Lobby => self.spec.lines.intro.clone(),
            Phase::Final => self.spec.lines.finale.clone(),
            _ => None,
        };
        let round = qref.map(|r| {
            let round = &self.spec.rounds[r.round];
            RoundView {
                number: r.round + 1,
                of: self.spec.rounds.len(),
                title: round.title.trim().to_string(),
                intro: if r.first_in_round { round.intro.clone() } else { None },
            }
        });
        let question = in_play.then(|| self.question_view(q));
        let time = (self.phase == Phase::Question).then(|| TimeView {
            deadline: self.deadline.map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true)),
            remaining_ms: match (self.deadline, self.paused_remaining_ms) {
                (Some(d), _) => (d - now).num_milliseconds().max(0),
                (None, Some(ms)) => ms,
                (None, None) => 0,
            },
            paused: self.deadline.is_none(),
        });
        let live: Vec<_> = self.players.iter().filter(|p| !p.removed).collect();
        let answered = in_play.then(|| {
            let names: Vec<String> =
                live.iter().filter(|p| self.record(q, &p.id).is_some()).map(|p| p.name.clone()).collect();
            AnsweredView { count: names.len(), of: live.len(), names: (is_screen || is_host).then_some(names) }
        });
        let reveal = (in_play && self.is_revealed(q)).then(|| self.reveal_view(q, is_screen || is_host));
        let board = self.leaderboard();
        let show_board = matches!(self.phase, Phase::Leaderboard | Phase::Final) || is_host;
        let limit = if is_screen { BIG_SCREEN_ROWS } else { usize::MAX };
        let leaderboard = show_board.then(|| Self::board_rows(&board, me_id, limit));
        let team_board =
            (show_board && self.spec.teams.mode == TeamMode::Teams).then(|| Self::team_rows(&self.team_board()));
        let lobby = (self.phase == Phase::Lobby)
            .then(|| live.iter().map(|p| LobbyRow { name: p.name.clone(), team: p.team.clone() }).collect());

        let me = me_id.and_then(|id| self.player(id)).map(|p| {
            let rec = in_play.then(|| self.record(q, &p.id)).flatten();
            let row = board.iter().find(|r| r.player == p.id);
            let result = (in_play && self.is_revealed(q)).then(|| match rec {
                Some(r) => ResultView { verdict: verdict_word(r.verdict), points: self.points_for(q, r) },
                None => ResultView { verdict: "no_answer", points: 0 },
            });
            MeView {
                name: p.name.clone(),
                team: p.team.clone(),
                removed: p.removed,
                score: if p.removed { 0 } else { self.score(&p.id) },
                rank: row.map(|r| r.rank).unwrap_or(0),
                of: board.len(),
                answered: rec.is_some(),
                my_answer: rec.map(|r| self.answer_display(q, &r.value)),
                result,
            }
        });

        let host = is_host.then(|| {
            let qq = in_play.then(|| self.question(q));
            HostView {
                answer_now: in_play.then(|| self.reveal_view(q, true)),
                accept: qq.map(|x| x.accepted().to_vec()).unwrap_or_default(),
                answers: if in_play {
                    self.answers_for(q)
                        .iter()
                        .map(|r| HostAnswer {
                            player: r.player.clone(),
                            name: self.player(&r.player).map(|p| p.name.clone()).unwrap_or_default(),
                            answer: self.answer_display(q, &r.value),
                            verdict: verdict_word(r.verdict),
                            points: self.points_for(q, r),
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                pending: self.pending_judgements(),
                players: self
                    .players
                    .iter()
                    .map(|p| HostPlayer {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        team: p.team.clone(),
                        connected: p.connected,
                        removed: p.removed,
                        score: if p.removed { 0 } else { self.score(&p.id) },
                    })
                    .collect(),
                can: self.host_can(),
            }
        });

        QuizView {
            kit: "quiz",
            viewer: who,
            title: self.spec.title.trim().to_string(),
            theme: self.spec.theme.clone(),
            phase: self.phase,
            teams: self.spec.teams.mode == TeamMode::Teams,
            line,
            round,
            question,
            time,
            answered,
            reveal,
            leaderboard,
            team_board,
            lobby,
            me,
            host,
        }
    }
}
