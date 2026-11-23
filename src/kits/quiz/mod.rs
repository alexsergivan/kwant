//! The Quiz kit (pa2): pocket.json v1, its validator, the game engine, text matching, per-viewer
//! views and the dry run. See autonomy/pocket-apps/pa2/schema.md and impl.md.

pub mod dry_run;
pub mod engine;
pub mod matching;
pub mod spec;
pub mod validate;
pub mod view;

use crate::kits::{ActionError, ActionOutcome, ActionSpec, Actor, Kit, KitGame, KitInfo, Problem, Retention, Role, Viewer};
use chrono::{DateTime, Utc};
use engine::{JudgeVerdict, QuizGame, Target};
use serde_json::Value;

pub const KIT_ID: &str = "quiz";

/// A complete, valid pocket.json (also in schema.md): a template the Quizmaster bot can start from.
/// Its source quotes are samples; a real quiz quotes the page it checked.
pub const EXAMPLE: &str = include_str!("example.json");

/// Every action the Quiz takes (Pocket API v1). `q` is the flat 0-based question index the
/// views show; `choice` answers name a position in the view's `options`.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "answer",
        by: &[Role::Guest],
        body: r#"{"q": 3, "value": 2 | "Paris" | 1066}"#,
        about: "answer the open question once (a second answer is ignored, a late one refused)",
    },
    ActionSpec { name: "team", by: &[Role::Guest], body: r#"{"name": "Quizzly Bears"}"#, about: "join or start a team (teams mode)" },
    ActionSpec { name: "start", by: &[Role::Host], body: "{}", about: "leave the lobby and open question 1" },
    ActionSpec {
        name: "next",
        by: &[Role::Host],
        body: "{}",
        about: "move on: lock the question, reveal, leaderboard, next question, final",
    },
    ActionSpec { name: "pause", by: &[Role::Host], body: "{}", about: "stop the question's timer" },
    ActionSpec { name: "resume", by: &[Role::Host], body: "{}", about: "restart a paused timer" },
    ActionSpec { name: "end", by: &[Role::Host], body: "{}", about: "end the quiz now and show the final" },
    ActionSpec {
        name: "judge",
        by: &[Role::Host, Role::Judge],
        body: r#"{"q": 3, "player": "17", "verdict": "yes" | "no" | "partial"}"#,
        about: "settle a near miss (the judge hook only settles near misses; the host may overrule any answer)",
    },
    ActionSpec {
        name: "adjust",
        by: &[Role::Host],
        body: r#"{"player": "17", "points": 1} or {"team": "Quizzly Bears", "points": -1}"#,
        about: "give or take points (+1 point)",
    },
    ActionSpec { name: "remove", by: &[Role::Host], body: r#"{"player": "17"}"#, about: "remove a player; their points leave the board" },
];

pub struct QuizKit;

impl QuizKit {
    pub const INFO: KitInfo = KitInfo {
        id: KIT_ID,
        name: "Quiz",
        spec_version: spec::VERSION,
        retention: Retention::AfterEnd { days: 7 },
        needs_pin: false,
        max_players: validate::MAX_PLAYERS,
    };
}

impl Kit for QuizKit {
    fn info(&self) -> KitInfo {
        Self::INFO
    }

    fn actions(&self) -> &'static [ActionSpec] {
        ACTIONS
    }

    fn validate(&self, spec_json: &str) -> Result<(), Vec<Problem>> {
        validate::parse(spec_json).map(|_| ())
    }

    fn start(&self, spec_json: &str, seed: u64, now: DateTime<Utc>) -> Result<Box<dyn KitGame>, Vec<Problem>> {
        let spec = validate::parse(spec_json)?;
        Ok(Box::new(QuizGame::new(spec, seed, now)))
    }

    fn restore(&self, spec_json: &str, saved: &Value) -> Result<Box<dyn KitGame>, String> {
        let spec = validate::parse(spec_json)
            .map_err(|p| format!("the stored spec no longer validates: {}", p.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("; ")))?;
        Ok(Box::new(QuizGame::restore(spec, saved)?))
    }

    fn dry_run(&self, spec_json: &str, players: usize, drop: usize, seed: u64) -> Result<Value, Vec<Problem>> {
        let spec = validate::parse(spec_json)?;
        let opts = dry_run::DryRunOptions { players, drop, seed, ..Default::default() };
        let report = dry_run::run(&spec, &opts);
        Ok(serde_json::to_value(report).unwrap_or(Value::Null))
    }
}

fn str_field<'a>(body: &'a Value, key: &str) -> Result<&'a str, ActionError> {
    body.get(key).and_then(Value::as_str).ok_or_else(|| ActionError::BadInput(format!("`{key}` is missing")))
}

fn q_field(body: &Value) -> Result<usize, ActionError> {
    body.get("q").and_then(Value::as_u64).map(|q| q as usize).ok_or_else(|| ActionError::BadInput("`q` is missing".into()))
}

/// Refuses an action this actor's role may not send.
fn check_role(name: &str, actor: &Actor) -> Result<(), ActionError> {
    let spec = ACTIONS.iter().find(|a| a.name == name).ok_or_else(|| ActionError::UnknownAction(name.to_string()))?;
    if spec.by.contains(&actor.role()) {
        Ok(())
    } else {
        Err(ActionError::NotAllowed(format!("`{name}` isn't yours to send")))
    }
}

impl QuizGame {
    /// The named-action entry point pa1 calls.
    pub fn act(&mut self, actor: &Actor, name: &str, body: &Value, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        check_role(name, actor)?;
        match (name, actor) {
            ("answer", Actor::Guest(id)) => {
                let value = body.get("value").ok_or_else(|| ActionError::BadInput("`value` is missing".into()))?;
                self.answer(id, q_field(body)?, value, now)
            }
            ("team", Actor::Guest(id)) => self.set_team(id, str_field(body, "name")?),
            ("start", _) => self.start(now),
            ("next", _) => self.next(now),
            ("pause", _) => self.pause(now),
            ("resume", _) => self.resume(now),
            ("end", _) => self.end(now),
            ("judge", _) => {
                let verdict = str_field(body, "verdict")?;
                let verdict = JudgeVerdict::parse(verdict)
                    .ok_or_else(|| ActionError::BadInput("a verdict is exactly \"yes\", \"no\" or \"partial\"".into()))?;
                self.judge(matches!(actor, Actor::Host), q_field(body)?, str_field(body, "player")?, verdict)
            }
            ("adjust", _) => {
                let points = body.get("points").and_then(Value::as_i64).ok_or_else(|| ActionError::BadInput("`points` is missing".into()))?;
                let target = match (body.get("player").and_then(Value::as_str), body.get("team").and_then(Value::as_str)) {
                    (Some(p), None) => Target::Player(p.to_string()),
                    (None, Some(t)) => Target::Team(t.to_string()),
                    _ => return Err(ActionError::BadInput("name either a `player` or a `team`".into())),
                };
                self.adjust(target, points)
            }
            ("remove", _) => self.remove(str_field(body, "player")?),
            _ => Err(ActionError::NotAllowed(format!("`{name}` isn't yours to send"))),
        }
    }
}

impl KitGame for QuizGame {
    fn join(&mut self, player: &str, name: &str, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        QuizGame::join(self, player, name, now)
    }
    fn set_connected(&mut self, player: &str, connected: bool, _now: DateTime<Utc>) -> bool {
        QuizGame::set_connected(self, player, connected)
    }
    fn action(&mut self, actor: &Actor, name: &str, body: &Value, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError> {
        self.act(actor, name, body, now)
    }
    fn tick(&mut self, now: DateTime<Utc>) -> bool {
        QuizGame::tick(self, now)
    }
    fn next_deadline(&self) -> Option<DateTime<Utc>> {
        QuizGame::next_deadline(self)
    }
    fn view(&self, viewer: &Viewer, now: DateTime<Utc>) -> Value {
        serde_json::to_value(QuizGame::view(self, viewer, now)).unwrap_or(Value::Null)
    }
    fn save(&self) -> Value {
        QuizGame::save(self)
    }
    fn ended(&self) -> bool {
        QuizGame::ended(self)
    }
}
