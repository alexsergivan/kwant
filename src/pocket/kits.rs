//! The kit registry (Pocket API v1). A kit is a deterministic engine the core drives: it checks a
//! spec, keeps its own state as JSON (saved by the core after every change), answers named
//! actions, and draws one view per viewer. The core never sends a kit's whole state to anyone:
//! each viewer gets only what `view(viewer)` returns for it, over `state` and over `ws` alike.
//!
//! Adding a kit: implement [`Kit`] in its own module (the Quiz is `crate::kits`, pa2) and add one
//! line to [`kit`]. Later versions of the API add to these shapes and never change them.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{json, Value};

/// Players a pocket takes unless its kit asks for fewer (quick tunnels cap requests in flight).
pub const DEFAULT_MAX_PLAYERS: usize = 60;
/// The most a kit may raise [`Kit::player_ceiling`] to (a class quiz, pa11). A pocket over
/// [`DEFAULT_MAX_PLAYERS`] is never offered a quick tunnel: it needs an address that stays the
/// same (own https, Funnel or Same Wi-Fi).
pub const STABLE_MAX_PLAYERS: usize = 120;

/// How long a kit keeps guest data (plan section 8 rule 11). The sweeper deletes the pocket, its
/// players, keys and events once `expires_at` passes; deleting is final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// `days` after the pocket ends (games: 7).
    AfterEnd { days: i64 },
    /// `days` after the first date in the spec among `fields` (Secret Santa: the reveal date,
    /// else the gift date, + 7). With none of them set, it behaves like `AfterEnd`.
    AfterDate { fields: &'static [&'static str], days: i64 },
    /// At the end (a class quiz keeps nothing).
    AtEnd,
    /// pa13a front desk: each guest's data goes `days` after that guest's last message (the kit's
    /// [`Kit::idle_players`], swept once a minute); the pocket itself `days` after it ends.
    AfterLastMessage { days: i64 },
}

impl Retention {
    /// When the data goes, if that is known yet: at creation `ended_at` is `None`.
    pub fn expires_at(&self, spec: &Value, ended_at: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        match *self {
            Retention::AfterEnd { days } | Retention::AfterLastMessage { days } => ended_at.map(|t| t + Duration::days(days)),
            Retention::AtEnd => ended_at,
            Retention::AfterDate { fields, days } => match fields.iter().find_map(|f| spec_date(spec.get(*f)?)) {
                Some(date) => Some(date + Duration::days(days)),
                None => ended_at.map(|t| t + Duration::days(days)),
            },
        }
    }

    /// The JSON the guest page and the host card show before anyone types.
    pub fn describe(&self) -> Value {
        match *self {
            Retention::AfterEnd { days } => json!({ "kind": "after_end", "days": days }),
            Retention::AfterDate { fields, days } => json!({ "kind": "after_date", "fields": fields, "days": days }),
            Retention::AtEnd => json!({ "kind": "at_end" }),
            Retention::AfterLastMessage { days } => json!({ "kind": "after_last_message", "days": days }),
        }
    }
}

/// A date in a spec: RFC 3339 (`2026-12-18T19:00:00Z`) or a plain day (`2026-12-18`, end of day UTC).
pub fn spec_date(v: &Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?.and_hms_opt(23, 59, 59).map(|t| t.and_utc())
}

/// What a kit declares for one spec (a kit may differ by spec: a class quiz keeps nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct KitRules {
    pub retention: Retention,
    /// Joining picks a 4-digit PIN and rejoining needs it (Secret Santa).
    pub needs_pin: bool,
    /// At most [`Kit::player_ceiling`] ([`DEFAULT_MAX_PLAYERS`] unless the kit raises it); the
    /// core clamps a bigger number.
    pub max_players: usize,
    /// Action names a guest may send; anything else is a 400.
    pub guest_actions: &'static [&'static str],
    /// Action names the host remote may send (for example `advance`); anything else is a 400.
    pub host_actions: &'static [&'static str],
}

/// Who a view is for. Each gets its own view; there is never one shared blob a page filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    Guest(String),
    Host,
    BigScreen,
}

impl Viewer {
    pub fn player_id(&self) -> Option<&str> {
        match self {
            Viewer::Guest(id) => Some(id),
            _ => None,
        }
    }
}

/// A player as a kit sees it (no PIN, no keys).
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub team: Option<String>,
    pub joined_at: DateTime<Utc>,
}

/// What a kit gets for one call: the spec, the players still in, the time, and somewhere to put
/// events the core saves with the state (deleted with the pocket).
pub struct KitCtx<'a> {
    pub spec: &'a Value,
    pub players: &'a [Player],
    pub now: DateTime<Utc>,
    pub events: Vec<KitEvent>,
}

impl<'a> KitCtx<'a> {
    pub fn new(spec: &'a Value, players: &'a [Player], now: DateTime<Utc>) -> Self {
        Self { spec, players, now, events: Vec::new() }
    }

    pub fn player(&self, id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn event(&mut self, player_id: Option<&str>, kind: &str, payload: Value) {
        self.events.push(KitEvent { player_id: player_id.map(str::to_string), kind: kind.to_string(), payload });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct KitEvent {
    pub player_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

/// Why a kit refused an action: a 400 (bad body), a 409 (not now), or (pa13a) its own status
/// and stable code, with fields merged into the error body (`retry_at`).
#[derive(Debug, Clone, PartialEq)]
pub enum KitError {
    BadInput(String),
    NotNow(String),
    Coded { status: u16, code: &'static str, message: String, extra: Value },
}

pub trait Kit: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn rules(&self, spec: &Value) -> KitRules;
    /// `Err` is a plain sentence for the bot or the person (shown as is).
    fn validate_spec(&self, spec: &Value) -> Result<(), String>;
    /// The pocket's title from its spec (shown on the card and the guest page).
    fn title(&self, spec: &Value) -> String {
        spec.get("title").and_then(Value::as_str).unwrap_or("").trim().chars().take(80).collect()
    }
    fn init(&self, spec: &Value) -> Value;
    /// May someone with this name (its lower-case key) join now? Called before the player is
    /// saved, with the players already in. Secret Santa refuses after the draw unless the host
    /// let that name in late. `NotNow` is a 409 `not_now`, `BadInput` a 400 `bad_action`.
    fn admit(&self, _ctx: &KitCtx, _state: &Value, _name_key: &str) -> Result<(), KitError> {
        Ok(())
    }
    fn on_join(&self, _ctx: &mut KitCtx, _state: &mut Value, _player: &Player) {}
    /// May the host remove this player now? Secret Santa refuses after the draw (it would
    /// change someone else's match). Same errors as [`Kit::admit`].
    fn may_remove(&self, _ctx: &KitCtx, _state: &Value, _player_id: &str) -> Result<(), KitError> {
        Ok(())
    }
    fn on_remove(&self, _ctx: &mut KitCtx, _state: &mut Value, _player_id: &str) {}
    /// A named action from `actor` (its name is already one the rules list for that actor).
    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError>;
    /// Called about once a second while someone is watching; `true` when the state changed
    /// (a timer ran out). Kits without timers keep the default.
    fn tick(&self, _ctx: &mut KitCtx, _state: &mut Value) -> bool {
        false
    }
    /// Host actions that still work after the pocket ended, while its data is kept (pa14b: the
    /// Wall's `set_best`, so the host can show the best 30 picked the morning after). None by default.
    fn host_actions_after_end(&self) -> &'static [&'static str] {
        &[]
    }
    /// `true` when the kit's timers must run even while nobody watches (a draw or a reveal
    /// weeks away): the sweeper then ticks its open pockets once a minute.
    fn ticks_unwatched(&self) -> bool {
        false
    }
    /// `true` when guests come back over days or weeks, so the link must keep working: such a
    /// pocket is never offered a quick tunnel (a new address per start), only own https, Funnel
    /// or Same Wi-Fi (pa6).
    fn stable_address_only(&self) -> bool {
        false
    }
    /// The most players this spec may take (pa11): [`DEFAULT_MAX_PLAYERS`], or up to
    /// [`STABLE_MAX_PLAYERS`] for a kit that allows more (a class quiz). The core clamps
    /// `rules().max_players` to it, and a pocket over 60 gets no quick tunnel.
    fn player_ceiling(&self, _spec: &Value) -> usize {
        DEFAULT_MAX_PLAYERS
    }
    /// `true` when joining takes a first name or nickname only (pa11 class quiz, plan rule 9):
    /// [`super::words::first_name_only`] refuses longer names and email or phone shapes.
    fn first_names_only(&self, _spec: &Value) -> bool {
        false
    }
    /// A plain line the guest page and the person's card show before anyone types (plan rules 9
    /// and 11), beside the retention line. `None` (the default): nothing beyond retention.
    fn notice(&self, _spec: &Value) -> Option<&'static str> {
        None
    }
    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value;
    /// What anyone with the link sees before joining (`info`, and the page's boot without a
    /// seat, as `view`): for example the Quiz's teams with counts, rounds and start time.
    /// Never a guest's name, answer, secret or match. `Null` (the default) means nothing.
    fn lobby(&self, _ctx: &KitCtx, _state: &Value) -> Value {
        Value::Null
    }
    /// Counts for the person's pocket card on the main port. Never names a guest's secret,
    /// answer or match: counts only.
    fn host_summary(&self, _ctx: &KitCtx, _state: &Value) -> Value {
        json!({})
    }
    /// pa7: `true` once the kit has reached its final state (the Quiz's final scores); the core
    /// then ends the pocket, which posts the results into the chat it came from. The default
    /// never finishes: the host's End or the closing time ends the pocket.
    fn finished(&self, _state: &Value) -> bool {
        false
    }
    /// pa7: the results the person's chat gets once the pocket has ended, in the `results` shape
    /// of the chat's pocket card (Pixel's pa4/impl.md "PocketBlock"): `{teams: [{name, players,
    /// points, palette, members}] (1-12, best first), margin, missed, near_misses,
    /// near_misses_accepted}`. Never a secret or a match. `Null` (the default): no results.
    fn results(&self, _ctx: &KitCtx, _state: &Value) -> Value {
        Value::Null
    }
    /// pa3b: a verdict from the bot's strict judge (`{q, player, verdict}`), sent only by the
    /// server through `pocket::judge`, never by a guest or host route. Kits without a judge keep
    /// the default refusal.
    fn judge(&self, _ctx: &mut KitCtx, _state: &mut Value, _body: &Value) -> Result<(), KitError> {
        Err(KitError::NotNow("this kit has no judge".into()))
    }
    /// pa13a: `true` when guests mustn't learn about each other at all, not even how many there
    /// are (the front desk): the public facts then carry no player count.
    fn private_guests(&self) -> bool {
        false
    }
    /// pa13a: `true` when a bot answers guests (the front desk): the core then refuses a spec
    /// without one (400 `needs_bot`) and fills `spec.answer_bot` and `spec.bot` at creation.
    fn answer_bot(&self) -> bool {
        false
    }
    /// pa13a: a step of the server's own work on the pocket (the front desk's bot turns), sent
    /// only by the server through `pocket::server_event`, never by a guest or host route. Its
    /// answer goes back to the server only.
    fn server_event(&self, _ctx: &mut KitCtx, _state: &mut Value, _name: &str, _body: &Value) -> Result<Value, KitError> {
        Err(KitError::NotNow("this kit takes no server events".into()))
    }
    /// pa13a: kwant threads the kit's state points at (a front desk guest's conversation with the
    /// bot). They are deleted with the guest or the pocket.
    fn owned_threads(&self, _state: &Value) -> Vec<String> {
        Vec::new()
    }
    /// pa13a: players whose data has expired by `ctx.now` (a front desk guest 7 days after their
    /// last message). The sweeper deletes each: their row, keys, events, threads and kit state.
    fn idle_players(&self, _ctx: &KitCtx, _state: &Value) -> Vec<String> {
        Vec::new()
    }
    /// pa13a: one part of the host's view by key (`guest:<id>`: a front desk conversation),
    /// for `host/state?view=<key>` and the card. `None`: no such part.
    fn host_detail(&self, _ctx: &KitCtx, _state: &Value, _key: &str) -> Option<Value> {
        None
    }
}

/// The kit registered under `id`. `test` exists only in debug builds or with
/// `KWNT_POCKET_TEST_KIT=1` (QA on a release image).
pub fn kit(id: &str) -> Option<&'static dyn Kit> {
    match id {
        "test" if test_kit_allowed() => Some(&TestKit),
        crate::kits::quiz::KIT_ID => Some(&super::quiz_kit::QuizPocket),
        super::santa::ID => Some(&super::santa::Santa),
        super::gather::ID => Some(&super::gather::Gather),
        super::wall::ID => Some(&super::wall::Wall),
        super::desk::ID => Some(&super::desk::Desk),
        _ => None,
    }
}

/// Every kit id this build can make a pocket with.
pub fn kit_ids() -> Vec<&'static str> {
    ["test", crate::kits::quiz::KIT_ID, super::santa::ID, super::gather::ID, super::wall::ID, super::desk::ID].into_iter().filter(|id| kit(id).is_some()).collect()
}

fn test_kit_allowed() -> bool {
    cfg!(debug_assertions) || std::env::var("KWNT_POCKET_TEST_KIT").is_ok_and(|v| v.trim() == "1")
}

/// A tiny kit for the core's tests. Spec: `title`, optional `pin` (bool), `max_players`,
/// `retention` (`after_end` | `after_date` | `at_end`), `days`, `reveal_at`, `gift_on`, and (pa7)
/// `final_round`: the round after which the kit is finished.
/// Guests `tap` and `set_secret {secret}` (a private field only its owner sees); the host can
/// `advance` the round.
pub struct TestKit;

const TEST_DATE_FIELDS: &[&str] = &["reveal_at", "gift_on"];

impl Kit for TestKit {
    fn id(&self) -> &'static str {
        "test"
    }

    fn rules(&self, spec: &Value) -> KitRules {
        let days = spec.get("days").and_then(Value::as_i64).unwrap_or(7);
        let retention = match spec.get("retention").and_then(Value::as_str) {
            Some("at_end") => Retention::AtEnd,
            Some("after_date") => Retention::AfterDate { fields: TEST_DATE_FIELDS, days },
            _ => Retention::AfterEnd { days },
        };
        KitRules {
            retention,
            needs_pin: spec.get("pin").and_then(Value::as_bool).unwrap_or(false),
            max_players: spec.get("max_players").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(DEFAULT_MAX_PLAYERS),
            guest_actions: &["tap", "set_secret"],
            host_actions: &["advance"],
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        if !spec.is_object() {
            return Err("The spec must be a JSON object.".into());
        }
        if spec.get("title").and_then(Value::as_str).is_none_or(|t| t.trim().is_empty()) {
            return Err("The spec needs a title.".into());
        }
        Ok(())
    }

    fn init(&self, spec: &Value) -> Value {
        json!({ "round": 0, "taps": {}, "secrets": {}, "final_round": spec.get("final_round").and_then(Value::as_i64) })
    }

    fn on_remove(&self, _ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        if let Some(s) = state["secrets"].as_object_mut() {
            s.remove(player_id);
        }
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        match (actor, name) {
            (Viewer::Guest(id), "tap") => {
                let n = state["taps"][id.as_str()].as_i64().unwrap_or(0) + 1;
                state["taps"][id.as_str()] = json!(n);
                ctx.event(Some(id), "tap", json!({}));
                Ok(())
            }
            (Viewer::Guest(id), "set_secret") => {
                let secret = body.get("secret").and_then(Value::as_str).ok_or_else(|| KitError::BadInput("secret is required".into()))?;
                if secret.chars().count() > 50 {
                    return Err(KitError::BadInput("secret is at most 50 characters".into()));
                }
                state["secrets"][id.as_str()] = json!(secret);
                Ok(())
            }
            (Viewer::Host, "advance") => {
                state["round"] = json!(state["round"].as_i64().unwrap_or(0) + 1);
                Ok(())
            }
            _ => Err(KitError::BadInput(format!("unknown action {name}"))),
        }
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let taps = |id: &str| state["taps"][id].as_i64().unwrap_or(0);
        let total: i64 = ctx.players.iter().map(|p| taps(&p.id)).sum();
        let players: Vec<Value> = ctx.players.iter().map(|p| json!({ "id": p.id, "name": p.name, "avatar": p.avatar, "taps": taps(&p.id) })).collect();
        match viewer {
            Viewer::Guest(id) => json!({
                "round": state["round"],
                "total_taps": total,
                "players": players,
                "me": { "taps": taps(id), "secret": state["secrets"][id.as_str()] },
            }),
            Viewer::Host => json!({
                "round": state["round"],
                "total_taps": total,
                "players": players,
                "secrets_set": state["secrets"].as_object().map_or(0, |s| s.len()),
            }),
            Viewer::BigScreen => json!({ "round": state["round"], "total_taps": total, "players": players }),
        }
    }

    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        json!({ "round": state["round"], "players": ctx.players.len() })
    }

    fn host_summary(&self, _ctx: &KitCtx, state: &Value) -> Value {
        json!({ "round": state["round"], "secrets_set": state["secrets"].as_object().map_or(0, |s| s.len()) })
    }

    /// With `final_round` in the spec, the pocket ends once the host has advanced to it.
    fn finished(&self, state: &Value) -> bool {
        state["final_round"].as_i64().is_some_and(|last| state["round"].as_i64().unwrap_or(0) >= last)
    }

    /// Each player is a team of one ("Team 1" first: the most taps), at most 12. The team's name
    /// is a label, the player's name only in `members` (blanked when the data expires); secrets
    /// never appear.
    fn results(&self, ctx: &KitCtx, state: &Value) -> Value {
        if ctx.players.is_empty() {
            return Value::Null;
        }
        let taps = |id: &str| state["taps"][id].as_i64().unwrap_or(0);
        let mut players: Vec<&Player> = ctx.players.iter().collect();
        players.sort_by_key(|p| std::cmp::Reverse(taps(&p.id)));
        let teams: Vec<Value> = players
            .iter()
            .take(12)
            .enumerate()
            .map(|(i, p)| json!({ "name": format!("Team {}", i + 1), "players": 1, "points": taps(&p.id), "palette": null, "members": [p.name] }))
            .collect();
        json!({ "teams": teams, "margin": null, "missed": [], "near_misses": null, "near_misses_accepted": null })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_dates() {
        let end = DateTime::parse_from_rfc3339("2026-12-01T10:00:00Z").unwrap().with_timezone(&Utc);
        let spec = json!({ "gift_on": "2026-12-18", "reveal_at": "2026-12-18T19:00:00Z" });
        assert_eq!(Retention::AfterEnd { days: 7 }.expires_at(&spec, None), None);
        assert_eq!(Retention::AfterEnd { days: 7 }.expires_at(&spec, Some(end)), Some(end + Duration::days(7)));
        assert_eq!(Retention::AtEnd.expires_at(&spec, Some(end)), Some(end));
        let santa = Retention::AfterDate { fields: TEST_DATE_FIELDS, days: 7 };
        // The reveal date comes first, then the gift date, then the end
        assert_eq!(santa.expires_at(&spec, None).unwrap().to_rfc3339(), "2026-12-25T19:00:00+00:00");
        assert_eq!(santa.expires_at(&json!({ "gift_on": "2026-12-18" }), None).unwrap().to_rfc3339(), "2026-12-25T23:59:59+00:00");
        assert_eq!(santa.expires_at(&json!({}), None), None);
        assert_eq!(santa.expires_at(&json!({}), Some(end)), Some(end + Duration::days(7)));
    }

    #[test]
    fn unknown_kits_are_none() {
        assert!(kit("quiz-nope").is_none());
        assert!(kit("test").is_some(), "debug builds have the test kit");
    }
}
