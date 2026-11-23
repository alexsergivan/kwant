//! Pocket app kits (pa2, plan: autonomy/pocket-apps/plan.md section 5).
//!
//! A kit is a deterministic engine plus a spec (pocket.json) that a bot fills in. The bot never
//! writes code that guests run, and no model sits in a live game's path: a near-miss text answer
//! waits for a strict `yes | no | partial` verdict that arrives from outside as an action.
//!
//! This module is pure: no database, no HTTP, no clock and no global RNG. Every call that needs
//! "now" takes it as an argument, and every random choice comes from a seed. The Pocket core
//! (pa1, `src/pocket/`) owns tables, guest keys, the guest port and the WebSocket; it stores a
//! game's [`KitGame::save`] JSON and calls the kit through the [`Kit`] and [`KitGame`] traits.
//!
//! Pocket API v1 rules a kit follows (pa1 brief, "Shaped as the toolkit"):
//! - per-viewer views: [`KitGame::view`] builds one viewer's JSON; there is never one shared blob
//!   that a page filters;
//! - named actions: [`Kit::actions`] lists every action name and who may send it; an unknown name
//!   is [`ActionError::UnknownAction`] (pa1 answers 400).

pub mod quiz;
pub mod rng;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A player's id as the Pocket core knows it (pa1's player row id, as text).
pub type PlayerId = String;

/// How long a kit keeps guest data (plan section 8 rule 11). pa1's registry maps this onto its
/// own `Retention` one to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Retention {
    /// Deleted this many days after the pocket ends (games: 7).
    AfterEnd { days: u32 },
    /// Deleted this many days after a date read from the spec (Secret Santa's reveal date).
    AfterDate { spec_field: &'static str, days: u32 },
    /// Deleted when the pocket ends (class quiz).
    AtEnd,
}

/// What a kit declares to the Pocket core's registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct KitInfo {
    /// The kit's id in pocket.json and in pa1's `pocket_apps.kit` column.
    pub id: &'static str,
    /// The name people see.
    pub name: &'static str,
    /// The pocket.json `version` this kit reads.
    pub spec_version: u32,
    pub retention: Retention,
    /// Whether rejoining needs a PIN chosen at join (Secret Santa: yes; Quiz: no).
    pub needs_pin: bool,
    /// The most players a pocket of this kit takes (pa1 default 60; a spec may lower it).
    pub max_players: u32,
}

/// Who is looking. Each viewer gets only its own view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "player")]
pub enum Viewer {
    /// A guest's phone, by player id.
    Guest(PlayerId),
    /// The host's remote (the person's own phone or kwant).
    Host,
    /// The big screen (TV or laptop at the venue): public, so it holds no secrets.
    BigScreen,
}

/// Who sends an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "player")]
pub enum Actor {
    Guest(PlayerId),
    Host,
    /// The judge hook (pa3): the bot settles a near miss, strict verdicts only.
    Judge,
}

/// The role an [`Actor`] has, for the action list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Guest,
    Host,
    Judge,
}

impl Actor {
    pub fn role(&self) -> Role {
        match self {
            Actor::Guest(_) => Role::Guest,
            Actor::Host => Role::Host,
            Actor::Judge => Role::Judge,
        }
    }
}

/// One named action a kit accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActionSpec {
    pub name: &'static str,
    /// Who may send it.
    pub by: &'static [Role],
    /// The JSON body, written out for pa1/pa4/pa5 docs.
    pub body: &'static str,
    /// What it does, in one line.
    pub about: &'static str,
}

/// What an accepted action did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "why")]
pub enum ActionOutcome {
    /// The game changed.
    Applied,
    /// Nothing changed and nothing is wrong (a second answer to the same question, a rejoin).
    Ignored(String),
}

/// Why an action was refused. The text is plain and safe to show the sender.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("{0}")]
    NotAllowed(String),
    #[error("{0}")]
    BadInput(String),
    #[error("{0}")]
    WrongPhase(String),
    #[error("time's up for that question")]
    TooLate,
    #[error("no such player")]
    UnknownPlayer,
    #[error("this player was removed by the host")]
    Removed,
    #[error("{0}")]
    Full(String),
}

/// One thing wrong with a spec, numbered so a bot can fix them one by one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// 1-based position in the list.
    pub n: usize,
    /// Where: "round 2, question 1", "teams", or none for the whole file.
    pub at: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.at {
            Some(at) => write!(f, "{}. {}: {}", self.n, at, self.message),
            None => write!(f, "{}. {}", self.n, self.message),
        }
    }
}

/// A kit, as pa1's registry holds it.
pub trait Kit: Send + Sync {
    fn info(&self) -> KitInfo;
    /// Every action name this kit accepts, and who may send it.
    fn actions(&self) -> &'static [ActionSpec];
    /// Checks a pocket.json; `Err` holds every problem found, numbered.
    fn validate(&self, spec_json: &str) -> Result<(), Vec<Problem>>;
    /// Starts a game in its lobby. Validates first.
    fn start(&self, spec_json: &str, seed: u64, now: DateTime<Utc>) -> Result<Box<dyn KitGame>, Vec<Problem>>;
    /// Rebuilds a game from its spec and a [`KitGame::save`] (after a restart).
    fn restore(&self, spec_json: &str, saved: &serde_json::Value) -> Result<Box<dyn KitGame>, String>;
    /// Plays the spec with `players` pretend players (dropping and rejoining `drop` of them
    /// mid-question) and returns the report as JSON. Validates first.
    fn dry_run(&self, spec_json: &str, players: usize, drop: usize, seed: u64) -> Result<serde_json::Value, Vec<Problem>>;
}

/// One running game. pa1 holds it behind a lock, calls [`KitGame::tick`] at
/// [`KitGame::next_deadline`], and pushes each viewer its own [`KitGame::view`] after a change.
pub trait KitGame: Send {
    /// A guest joins, or comes back (same id: nothing is lost, the outcome is `Ignored`).
    fn join(&mut self, player: &str, name: &str, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError>;
    /// A guest's connection opened or closed. Points and answers never depend on this.
    fn set_connected(&mut self, player: &str, connected: bool, now: DateTime<Utc>) -> bool;
    /// A named action; the body is checked by the kit.
    fn action(&mut self, actor: &Actor, name: &str, body: &serde_json::Value, now: DateTime<Utc>) -> Result<ActionOutcome, ActionError>;
    /// Runs timers up to `now`; true when something changed.
    fn tick(&mut self, now: DateTime<Utc>) -> bool;
    /// When [`KitGame::tick`] next has work to do, if ever.
    fn next_deadline(&self) -> Option<DateTime<Utc>>;
    /// What this viewer may see, as JSON.
    fn view(&self, viewer: &Viewer, now: DateTime<Utc>) -> serde_json::Value;
    /// The whole game state, for pa1 to store (the spec is stored separately).
    fn save(&self) -> serde_json::Value;
    /// True once the game reached its end.
    fn ended(&self) -> bool;
}

/// Every kit this build ships.
pub fn builtin() -> Vec<Box<dyn Kit>> {
    vec![Box::new(quiz::QuizKit)]
}

/// The kit with this id.
pub fn by_id(id: &str) -> Option<Box<dyn Kit>> {
    builtin().into_iter().find(|k| k.info().id == id)
}
