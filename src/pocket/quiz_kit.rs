//! The Quiz kit (pa2, `crate::kits::quiz`) as a Pocket core kit (pa3b): the `QuizPocket` adapter
//! pa2/impl.md "Registering with pa1" describes. Every call restores the game from the spec and
//! the saved JSON, runs it, and hands the new JSON back for the core to save.
//!
//! The judge's verdicts come in through [`Kit::judge`], which the core calls only from
//! `pocket::judge` (server side, the live judge in `server::quizmaster_live`): no guest or host
//! route reaches it, and the kit itself refuses a judge verdict on anything but a near miss.

use super::kits::{Kit, KitCtx, KitError, KitRules, Player, Retention, Viewer, DEFAULT_MAX_PLAYERS};
use crate::kits::quiz::engine::QuizGame;
use crate::kits::quiz::{validate, QuizKit, ACTIONS, KIT_ID};
use crate::kits::{self as qk, ActionError, Actor, Kit as _, Role};
use chrono::Utc;
use serde_json::{json, Value};

pub struct QuizPocket;

/// Guest and host action names, as `ACTIONS` lists them by role (a unit test keeps them equal).
const GUEST_ACTIONS: &[&str] = &["answer", "team"];
const HOST_ACTIONS: &[&str] = &["start", "next", "pause", "resume", "end", "judge", "adjust", "remove"];
/// The chat's results card takes at most this many lines.
const MAX_RESULT_LINES: usize = 12;

fn kit_error(e: ActionError) -> KitError {
    match e {
        ActionError::BadInput(_) | ActionError::UnknownAction(_) | ActionError::NotAllowed(_) => KitError::BadInput(e.to_string()),
        ActionError::WrongPhase(_) | ActionError::TooLate | ActionError::Full(_) | ActionError::Removed | ActionError::UnknownPlayer => {
            KitError::NotNow(e.to_string())
        }
    }
}

fn quiz_viewer(v: &Viewer) -> qk::Viewer {
    match v {
        Viewer::Guest(id) => qk::Viewer::Guest(id.clone()),
        Viewer::Host => qk::Viewer::Host,
        Viewer::BigScreen => qk::Viewer::BigScreen,
    }
}

/// The running game, or `None` when the saved state doesn't read (never expected: the core saved
/// it). Views then show nothing and actions say so.
fn restore(spec: &Value, state: &Value) -> Option<QuizGame> {
    let spec = validate::parse(&spec.to_string()).ok()?;
    QuizGame::restore(spec, state).ok()
}

fn restore_or_err(spec: &Value, state: &Value) -> Result<QuizGame, KitError> {
    restore(spec, state).ok_or_else(|| KitError::NotNow("this quiz's saved state doesn't read".into()))
}

impl QuizPocket {
    /// Runs one action as `actor` and writes the game back into `state`.
    fn run(ctx: &KitCtx, state: &mut Value, actor: &Actor, name: &str, body: &Value) -> Result<(), KitError> {
        let mut g = restore_or_err(ctx.spec, state)?;
        g.act(actor, name, body, ctx.now).map_err(kit_error)?;
        *state = g.save();
        Ok(())
    }

    /// The near misses waiting for the judge, as the host view lists them.
    pub fn pending(spec: &Value, state: &Value) -> Value {
        restore(spec, state).map_or(json!([]), |g| json!(g.pending_judgements()))
    }
}

impl Kit for QuizPocket {
    fn id(&self) -> &'static str {
        KIT_ID
    }

    fn rules(&self, spec: &Value) -> KitRules {
        let days = match QuizKit::INFO.retention {
            qk::Retention::AfterEnd { days } => i64::from(days),
            _ => 7,
        };
        let max = spec["settings"]["max_players"].as_u64().map_or(QuizKit::INFO.max_players as usize, |n| n as usize);
        KitRules {
            retention: Retention::AfterEnd { days },
            needs_pin: QuizKit::INFO.needs_pin,
            max_players: max.clamp(1, DEFAULT_MAX_PLAYERS),
            guest_actions: GUEST_ACTIONS,
            host_actions: HOST_ACTIONS,
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        QuizKit.validate(&spec.to_string()).map_err(|p| p.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n"))
    }

    fn init(&self, spec: &Value) -> Value {
        let seed = uuid::Uuid::new_v4().as_u64_pair().0;
        QuizKit.start(&spec.to_string(), seed, Utc::now()).map_or(Value::Null, |g| g.save())
    }

    fn on_join(&self, ctx: &mut KitCtx, state: &mut Value, player: &Player) {
        let Some(mut g) = restore(ctx.spec, state) else { return };
        if g.join(&player.id, &player.name, ctx.now).is_err() {
            return;
        }
        if let Some(team) = &player.team {
            // A team the kit refuses (full, or teams are off) leaves the player without one
            let _ = g.act(&Actor::Guest(player.id.clone()), "team", &json!({ "name": team }), ctx.now);
        }
        *state = g.save();
    }

    fn on_remove(&self, ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        let _ = Self::run(ctx, state, &Actor::Host, "remove", &json!({ "player": player_id }));
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let actor = match actor {
            Viewer::Guest(id) => Actor::Guest(id.clone()),
            Viewer::Host => Actor::Host,
            Viewer::BigScreen => return Err(KitError::BadInput("the big screen sends no actions".into())),
        };
        Self::run(ctx, state, &actor, name, body)
    }

    fn judge(&self, ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
        Self::run(ctx, state, &Actor::Judge, "judge", body)
    }

    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let Some(mut g) = restore(ctx.spec, state) else { return false };
        if !g.tick(ctx.now) {
            return false;
        }
        *state = g.save();
        true
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        restore(ctx.spec, state).map_or(Value::Null, |g| qk::KitGame::view(&g, &quiz_viewer(viewer), ctx.now))
    }

    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        restore(ctx.spec, state).map_or(Value::Null, |g| qk::KitGame::lobby(&g))
    }

    /// Counts only: the phase, players still in, answers in for the open question, near misses
    /// waiting.
    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        let Some(g) = restore(ctx.spec, state) else { return json!({}) };
        let v = qk::KitGame::view(&g, &qk::Viewer::Host, ctx.now);
        json!({
            "phase": v["phase"],
            "players": g.leaderboard().len(),
            "answered": v["answered"]["count"],
            "pending": g.pending_judgements().len(),
        })
    }

    fn finished(&self, state: &Value) -> bool {
        state["phase"] == json!("final")
    }

    /// Teams mode: one line per team (its name). Solo: one line per player, labelled "Player N"
    /// with the guest's name only in `members` (blanked when the data goes). Best first, at most 12.
    fn results(&self, ctx: &KitCtx, state: &Value) -> Value {
        let Some(g) = restore(ctx.spec, state) else { return Value::Null };
        let board = g.leaderboard();
        let lines: Vec<(String, usize, i64, Vec<String>)> = if g.spec().teams.mode == crate::kits::quiz::spec::TeamMode::Teams {
            g.team_board()
                .into_iter()
                .map(|t| {
                    let members = board.iter().filter(|r| r.team.as_deref() == Some(t.team.as_str())).map(|r| r.name.clone()).collect();
                    (t.team, t.members, t.score, members)
                })
                .collect()
        } else {
            board.iter().enumerate().map(|(i, r)| (format!("Player {}", i + 1), 1, r.score, vec![r.name.clone()])).collect()
        };
        if lines.is_empty() {
            return Value::Null;
        }
        let margin = match lines.as_slice() {
            [a, b, ..] => json!(a.2 - b.2),
            _ => Value::Null,
        };
        let teams: Vec<Value> = lines
            .into_iter()
            .take(MAX_RESULT_LINES)
            .map(|(name, players, points, members)| json!({ "name": name, "players": players, "points": points, "palette": null, "members": members }))
            .collect();
        json!({ "teams": teams, "margin": margin, "missed": [], "near_misses": null, "near_misses_accepted": null })
    }
}

/// The action names `ACTIONS` gives a role (the unit test below and the docs).
pub fn actions_for(role: Role) -> Vec<&'static str> {
    ACTIONS.iter().filter(|a| a.by.contains(&role)).map(|a| a.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kits::quiz::EXAMPLE;

    fn spec() -> Value {
        let mut v: Value = serde_json::from_str(EXAMPLE).unwrap();
        v["teams"] = json!({ "mode": "solo" });
        v
    }

    fn player(id: &str, name: &str) -> Player {
        Player { id: id.into(), name: name.into(), avatar: String::new(), team: None, joined_at: Utc::now() }
    }

    #[test]
    fn action_lists_match_the_quiz_kit() {
        assert_eq!(GUEST_ACTIONS, actions_for(Role::Guest).as_slice());
        assert_eq!(HOST_ACTIONS, actions_for(Role::Host).as_slice());
        assert!(!GUEST_ACTIONS.contains(&"judge"), "a guest never sends a verdict");
    }

    #[test]
    fn the_example_plays_through_the_core_shape() {
        let spec = spec();
        assert_eq!(QuizPocket.validate_spec(&spec), Ok(()));
        assert!(QuizPocket.validate_spec(&json!({ "version": 1, "kit": "quiz", "rounds": [] })).unwrap_err().starts_with("1. "));
        let rules = QuizPocket.rules(&spec);
        assert_eq!(rules.retention, Retention::AfterEnd { days: 7 });
        assert!(!rules.needs_pin);
        let mut state = QuizPocket.init(&spec);
        assert_eq!(state["phase"], json!("lobby"));
        let players = [player("1", "Ann"), player("2", "Bob")];
        let mut ctx = KitCtx::new(&spec, &players, Utc::now());
        for p in &players {
            QuizPocket.on_join(&mut ctx, &mut state, p);
        }
        QuizPocket.act(&mut ctx, &mut state, &Viewer::Host, "start", &json!({})).unwrap();
        assert_eq!(QuizPocket.view(&ctx, &state, &Viewer::Host)["phase"], json!("question"));
        assert_eq!(QuizPocket.host_summary(&ctx, &state)["players"], json!(2));
        // A guest can't send a host action, and the big screen sends nothing
        assert!(matches!(QuizPocket.act(&mut ctx, &mut state, &Viewer::Guest("1".into()), "next", &json!({})), Err(KitError::BadInput(_))));
        assert!(QuizPocket.act(&mut ctx, &mut state, &Viewer::BigScreen, "answer", &json!({})).is_err());
        // A judge verdict on an item that isn't a near miss is refused
        assert!(QuizPocket.judge(&mut ctx, &mut state, &json!({ "q": 0, "player": "1", "verdict": "yes" })).is_err());
        QuizPocket.act(&mut ctx, &mut state, &Viewer::Host, "end", &json!({})).unwrap();
        assert!(QuizPocket.finished(&state));
        let r = QuizPocket.results(&ctx, &state);
        assert_eq!(r["teams"].as_array().unwrap().len(), 2, "{r}");
        assert_eq!(r["teams"][0]["name"], json!("Player 1"), "a label, not the guest's name");
        let names: Vec<&str> = r["teams"].as_array().unwrap().iter().map(|t| t["members"][0].as_str().unwrap()).collect();
        assert!(names.contains(&"Ann") && names.contains(&"Bob"), "{r}");
    }
}
