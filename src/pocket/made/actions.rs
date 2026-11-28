//! The made kit's server actions (pa12 L4, plan section 2): `answer`, `vote`, `draw`,
//! `timer_start`/`timer_stop`, `lock`/`unlock`, `turn_next`/`turn_skip`, `reveal`, `step` and
//! `adjust`, beside L2's `put`/`take_back` in [`super::collections`]. A spec only configures
//! these (its `actions` and `steps`); it can never name a new one.
//!
//! Everything that needs judgement is worked out on the box: an answer's score against the spec's
//! `secrets` (toolkit `match_text`), a vote's tally (toolkit `count_one_pick`/`count_ranked`), a
//! draw (toolkit `draw_loop`/`insert_into_loop`/`Rng`). A guest sends only a guess, a pick or a
//! ranking; any other field in the body (`points`, `score`, `answer_key`, ...) is refused.
//!
//! What a viewer sees of each part, built by [`view_parts`]:
//! - `revealed`: the texts of the secret groups that are out (the host's `reveal`, a `reveal`
//!   step, or `reveal_at` passing); nothing of any other secret, ever.
//! - `scores`: points for answers to revealed groups only, plus the host's `adjust`; so a score
//!   never tells anyone whether an unrevealed guess was right.
//! - `vote`: the guest's own ballot, the number of voters and the tally (or not, until the
//!   reveal, with `tally: "after_reveal"`); never anyone else's ballot.
//! - `draw`: a guest sees only whom they drew; the host and the big screen see only that names
//!   are drawn and how many.
//! - `timer`, `turn` and `step`: the same for everyone.

use super::super::kits::{spec_date, KitCtx, KitError, Player, Viewer};
use super::super::toolkit::{self, Block, Outcome, Rng, TextMatch};
use super::super::words;
use super::collections;
use super::spec::valid_name;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{json, Map, Value};

/// The actions a guest may send (beside L2's `put` and `take_back`).
pub const GUEST: &[&str] = &["answer", "vote"];
/// The actions the host may send (beside L2's `put`).
pub const HOST: &[&str] = &["draw", "timer_start", "timer_stop", "lock", "unlock", "turn_next", "turn_skip", "reveal", "step", "adjust"];

/// The spec's `actions` keys.
pub const CONFIGS: &[&str] = &["answer", "vote", "draw", "timer", "turns", "reveal"];
pub const STEP_KEYS: &[&str] = &["say", "open", "lock", "timer", "reveal"];
pub const MAX_STEPS: usize = 50;
pub const SAY_MAX: usize = 120;
pub const TIMER_MIN: u64 = 5;
pub const TIMER_MAX: u64 = 3600;
pub const MAX_POINTS: u64 = 100;
/// The host's `adjust` is capped per tap like the quiz's (re-exported by the toolkit).
pub use super::super::toolkit::MAX_ADJUST;
/// Adjustments kept per pocket (each is a tap).
pub const MAX_ADJUSTMENTS: usize = 1000;
pub const MAX_GROUPS: usize = 20;
pub const MAX_ITEMS: usize = 50;
pub const SECRET_MAX: usize = 200;
pub const MAX_ACCEPT: usize = 10;
pub const MAX_APART: usize = 100;
/// The longest guess `answer` keeps when its collection's `text` field allows more.
pub const GUESS_MAX: usize = 200;

fn bad(m: impl Into<String>) -> KitError {
    KitError::BadInput(m.into())
}

fn not_now(m: impl Into<String>) -> KitError {
    KitError::NotNow(m.into())
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

fn config<'a>(spec: &'a Value, name: &str) -> Option<&'a Value> {
    spec["actions"].get(name).filter(|v| v.is_object())
}

/// The body must be an object (or nothing) with only these keys. A guest-sent `points`, `score`
/// or `answer_key` is refused here, before anything else is read.
fn only<'b>(body: &'b Value, keys: &[&str], action: &str) -> Result<&'b Map<String, Value>, KitError> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    let o = match body {
        Value::Null => EMPTY.get_or_init(Map::new),
        Value::Object(o) => o,
        _ => return Err(bad(format!("{action}: send an object."))),
    };
    if let Some(k) = o.keys().find(|k| !keys.contains(&k.as_str())) {
        let takes = if keys.is_empty() { "takes nothing".to_string() } else { format!("takes only {}", keys.join(", ")) };
        return Err(bad(format!("{action} {takes}; {:?} isn't one of them.", short(k))));
    }
    Ok(o)
}

// ---- secrets ----

/// One secret item: the text and the other spellings accepted for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Secret {
    pub text: String,
    pub accept: Vec<String>,
}

fn item(v: &Value) -> Option<Secret> {
    match v {
        Value::String(s) => Some(Secret { text: s.clone(), accept: vec![] }),
        Value::Object(o) => Some(Secret {
            text: o.get("text")?.as_str()?.to_string(),
            accept: o.get("accept").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        }),
        _ => None,
    }
}

/// The items of one secret group: a text, a `{text, accept}` or a list of them.
pub fn group(spec: &Value, name: &str) -> Vec<Secret> {
    match spec["secrets"].get(name) {
        Some(Value::Array(a)) => a.iter().filter_map(item).collect(),
        Some(v) => item(v).into_iter().collect(),
        None => vec![],
    }
}

/// `"secrets.answers"` or `"answers"` names the group `answers`.
fn group_name(against: &str) -> &str {
    against.strip_prefix("secrets.").unwrap_or(against)
}

/// The groups a full reveal (the host's `reveal` with no body, or `reveal_at`) publishes: the
/// reveal config's `secrets`, else the group `answer` is played against.
fn full_reveal_groups(spec: &Value) -> Vec<String> {
    if let Some(r) = config(spec, "reveal").and_then(|r| r["secrets"].as_array()) {
        return r.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
    }
    answer_cfg(spec).map(|a| vec![a.group.to_string()]).unwrap_or_default()
}

fn reveal_at_passed(spec: &Value, now: DateTime<Utc>) -> bool {
    spec.get("reveal_at").and_then(spec_date).is_some_and(|t| t <= now)
}

/// The secret groups that are out now. A poll may come before the tick that marks `reveal_at`,
/// so the time is checked here as well.
pub fn shown(spec: &Value, state: &Value, now: DateTime<Utc>) -> Vec<String> {
    let mut out: Vec<String> = state["secrets_shown"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if reveal_at_passed(spec, now) {
        for g in full_reveal_groups(spec) {
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    out
}

// ---- locks and timers ----

/// What `lock`, `unlock` and `timer_start` act on: a collection, or the vote.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Coll(String),
    Vote,
}

fn target(spec: &Value, o: &Map<String, Value>, action: &str) -> Result<Target, KitError> {
    match (o.get("collection"), o.get("vote")) {
        (Some(c), None) => {
            let name = c.as_str().ok_or_else(|| bad(format!("{action}: collection is the name of a collection.")))?;
            collections::find(spec, name).ok_or_else(|| bad(format!("There's no collection called {:?}.", short(name))))?;
            Ok(Target::Coll(name.to_string()))
        }
        (None, Some(Value::Bool(true))) => {
            config(spec, "vote").ok_or_else(|| bad("This app has no vote."))?;
            Ok(Target::Vote)
        }
        _ => Err(bad(format!("{action}: say which collection (collection), or vote: true."))),
    }
}

fn target_json(t: &Target) -> Value {
    match t {
        Target::Coll(c) => json!({ "collection": c }),
        Target::Vote => json!({ "vote": true }),
    }
}

fn target_of(v: &Value) -> Option<Target> {
    if v["vote"] == json!(true) {
        return Some(Target::Vote);
    }
    v["collection"].as_str().map(|c| Target::Coll(c.to_string()))
}

fn set_lock(state: &mut Value, t: &Target, on: bool) {
    match t {
        Target::Coll(c) => {
            if !state["locked"].is_object() {
                state["locked"] = json!({});
            }
            state["locked"][c.as_str()] = json!(on);
        }
        Target::Vote => state["vote_locked"] = json!(on),
    }
}

fn timer_end(state: &Value) -> Option<(Target, DateTime<Utc>)> {
    let t = &state["timer"];
    let end = t["ends_at"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok())?.with_timezone(&Utc);
    Some((target_of(t)?, end))
}

/// Locked now: the flag, or a timer on it that has run out (a poll may come before the tick).
fn is_locked(state: &Value, t: &Target, now: DateTime<Utc>) -> bool {
    let flag = match t {
        Target::Coll(c) => collections::locked(state, c),
        Target::Vote => state["vote_locked"].as_bool().unwrap_or(false),
    };
    flag || timer_end(state).is_some_and(|(tt, end)| &tt == t && end <= now)
}

/// A collection is locked now (for L2's `put`/`take_back` and the views).
pub fn coll_locked(state: &Value, name: &str, now: DateTime<Utc>) -> bool {
    is_locked(state, &Target::Coll(name.to_string()), now)
}

fn seconds(v: Option<&Value>, what: &str) -> Result<Option<u64>, KitError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().filter(|n| (TIMER_MIN..=TIMER_MAX).contains(n)).map(Some).ok_or_else(|| bad(format!("{what}: send a whole number of seconds from {TIMER_MIN} to {TIMER_MAX}."))),
    }
}

fn start_timer(ctx: &KitCtx, state: &mut Value, t: &Target, secs: u64) {
    let end = ctx.now + Duration::seconds(secs as i64);
    let mut v = target_json(t);
    v["seconds"] = json!(secs);
    v["ends_at"] = json!(end.to_rfc3339_opts(SecondsFormat::Secs, true));
    state["timer"] = v;
}

pub fn timer_start(ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
    let o = only(body, &["collection", "vote", "seconds"], "timer_start")?;
    let t = target(ctx.spec, o, "timer_start")?;
    let secs = match seconds(o.get("seconds"), "seconds")? {
        Some(s) => s,
        None => seconds(config(ctx.spec, "timer").and_then(|c| c.get("seconds")), "seconds")?.ok_or_else(|| bad("timer_start: say how long (seconds)."))?,
    };
    start_timer(ctx, state, &t, secs);
    Ok(())
}

pub fn timer_stop(state: &mut Value, body: &Value) -> Result<(), KitError> {
    only(body, &[], "timer_stop")?;
    if state["timer"].is_null() {
        return Err(not_now("No timer is running."));
    }
    state["timer"] = Value::Null;
    Ok(())
}

pub fn lock(ctx: &mut KitCtx, state: &mut Value, body: &Value, on: bool) -> Result<(), KitError> {
    let name = if on { "lock" } else { "unlock" };
    let o = only(body, &["collection", "vote"], name)?;
    let t = target(ctx.spec, o, name)?;
    set_lock(state, &t, on);
    // Unlocking also ends a timer that has run out on it, or it would stay locked
    if !on && timer_end(state).is_some_and(|(tt, end)| tt == t && end <= ctx.now) {
        state["timer"] = Value::Null;
    }
    Ok(())
}

// ---- answer ----

struct AnswerCfg<'a> {
    collection: &'a str,
    group: &'a str,
    points: i64,
}

fn answer_cfg(spec: &Value) -> Option<AnswerCfg<'_>> {
    let a = config(spec, "answer")?;
    Some(AnswerCfg {
        collection: a["collection"].as_str()?,
        group: group_name(a["against"].as_str()?),
        points: a["points"].as_u64().unwrap_or(1).min(MAX_POINTS) as i64,
    })
}

/// `answer {q?, text}`: the guest's guess for item `q` (default 0) of the secret group the spec
/// plays against, kept in the answer collection (one row per guest and item, overwritten).
/// Refused once that group is out or while the collection is locked.
pub fn answer(ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, body: &Value) -> Result<(), KitError> {
    let Viewer::Guest(who) = actor else {
        return Err(bad("Only a guest answers."));
    };
    let o = only(body, &["q", "text"], "answer")?;
    let cfg = answer_cfg(ctx.spec).ok_or_else(|| bad("This app has no answer action."))?;
    let items = group(ctx.spec, cfg.group);
    if items.is_empty() {
        return Err(not_now("There's nothing to answer yet."));
    }
    let q = match o.get("q") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().filter(|n| (*n as usize) < items.len()).ok_or_else(|| bad(format!("q: send a whole number from 0 to {}.", items.len() - 1)))?,
    };
    if shown(ctx.spec, state, ctx.now).iter().any(|g| g == cfg.group) {
        return Err(not_now("The answers are out, so answering has closed."));
    }
    if coll_locked(state, cfg.collection, ctx.now) {
        return Err(not_now("Answering is closed right now."));
    }
    let c = collections::find(ctx.spec, cfg.collection).ok_or_else(|| bad("This app has no answer action."))?;
    let fields = c.fields.cloned().unwrap_or_default();
    let max = fields.get("text").and_then(|f| f["max"].as_u64()).unwrap_or(GUESS_MAX as u64).min(GUESS_MAX as u64) as usize;
    let text = o.get("text").and_then(Value::as_str).ok_or_else(|| bad("text: send your answer."))?;
    let text = words::clean_text(text, max).map_err(|e| bad(format!("text: {}", e.message())))?;
    let mut values = json!({ "text": text });
    if fields.contains_key("q") {
        values["q"] = json!(q);
    }
    let at = ctx.now.to_rfc3339_opts(SecondsFormat::Secs, true);
    if !state["rows"].is_object() {
        state["rows"] = json!({});
    }
    if !state["rows"][cfg.collection].is_array() {
        state["rows"][cfg.collection] = json!([]);
    }
    let list = state["rows"][cfg.collection].as_array_mut().expect("just made a list");
    let same = |r: &Value| r["owner"].as_str() == Some(who.as_str()) && r["values"]["q"].as_u64().unwrap_or(0) == q;
    if let Some(row) = list.iter_mut().find(|r| same(r)) {
        row["values"] = values;
        row["at"] = json!(at);
    } else {
        list.push(json!({ "id": collections::new_row_id(), "owner": who, "values": values, "at": at }));
    }
    Ok(())
}

/// Points each player has: answers to revealed groups that match (toolkit `match_text`; in v1 a
/// near miss scores nothing and the host can `adjust`), plus the host's adjustments. `None` when
/// the app has no answer action and nobody was adjusted.
fn scores(spec: &Value, state: &Value, players: &[Player], now: DateTime<Utc>) -> Option<Value> {
    let adjust = state["adjust"].as_array().map_or(&[][..], Vec::as_slice);
    let cfg = answer_cfg(spec);
    if cfg.is_none() && adjust.is_empty() {
        return None;
    }
    let mut points: Vec<(&str, i64)> = players.iter().map(|p| (p.id.as_str(), 0)).collect();
    if let Some(cfg) = cfg.filter(|c| shown(spec, state, now).iter().any(|g| g == c.group)) {
        let items = group(spec, cfg.group);
        for r in collections::rows(state, cfg.collection) {
            let q = r["values"]["q"].as_u64().unwrap_or(0) as usize;
            let (Some(item), Some(guess)) = (items.get(q), r["values"]["text"].as_str()) else { continue };
            if toolkit::match_text(guess, &item.text, &item.accept) == TextMatch::Correct {
                if let Some(p) = points.iter_mut().find(|(id, _)| Some(*id) == r["owner"].as_str()) {
                    p.1 += cfg.points;
                }
            }
        }
    }
    for a in adjust {
        if let Some(p) = points.iter_mut().find(|(id, _)| Some(*id) == a["player"].as_str()) {
            p.1 += a["points"].as_i64().unwrap_or(0);
        }
    }
    Some(Value::Array(points.into_iter().map(|(id, n)| json!({ "id": id, "points": n })).collect()))
}

// ---- adjust ----

/// `adjust {player, points}`: the host gives or takes points, at most [`MAX_ADJUST`] per tap.
pub fn adjust(ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
    let o = only(body, &["player", "points"], "adjust")?;
    let player = o.get("player").and_then(Value::as_str).filter(|p| ctx.player(p).is_some()).ok_or_else(|| bad("player: there's no such player."))?;
    let points = o.get("points").and_then(Value::as_i64).filter(|n| *n != 0 && n.abs() <= MAX_ADJUST).ok_or_else(|| bad(format!("points: give between -{MAX_ADJUST} and {MAX_ADJUST} points, not 0.")))?;
    if !state["adjust"].is_array() {
        state["adjust"] = json!([]);
    }
    let list = state["adjust"].as_array_mut().expect("just made a list");
    if list.len() >= MAX_ADJUSTMENTS {
        return Err(not_now(format!("That's {MAX_ADJUSTMENTS} changes to the scores already.")));
    }
    list.push(json!({ "player": player, "points": points }));
    Ok(())
}

// ---- vote ----

struct VoteCfg<'a> {
    collection: &'a str,
    ranked: bool,
    tally_after_reveal: bool,
}

fn vote_cfg(spec: &Value) -> Option<VoteCfg<'_>> {
    let v = config(spec, "vote")?;
    Some(VoteCfg {
        collection: v["collection"].as_str()?,
        ranked: v["count"].as_str() == Some("ranked"),
        tally_after_reveal: v["tally"].as_str() == Some("after_reveal"),
    })
}

/// `vote {pick}` (one pick) or `vote {ranking: [..]}` (ranked): one ballot per guest, over the
/// rows of the vote's collection, overwritten until the vote is locked. Ballots stay in the
/// state and never appear in any view.
pub fn vote(ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, body: &Value) -> Result<(), KitError> {
    let Viewer::Guest(who) = actor else {
        return Err(bad("Only a guest votes."));
    };
    let cfg = vote_cfg(ctx.spec).ok_or_else(|| bad("This app has no vote."))?;
    let o = only(body, if cfg.ranked { &["ranking"] } else { &["pick"] }, "vote")?;
    if is_locked(state, &Target::Vote, ctx.now) {
        return Err(not_now("Voting has closed."));
    }
    let options: Vec<&str> = collections::rows(state, cfg.collection).iter().filter_map(|r| r["id"].as_str()).collect();
    let ballot: Vec<String> = if cfg.ranked {
        let list = o.get("ranking").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= options.len()).ok_or_else(|| bad("ranking: send the ids of the options, best first."))?;
        let mut out: Vec<String> = Vec::new();
        for v in list {
            let id = v.as_str().filter(|id| options.contains(id)).ok_or_else(|| bad("ranking: there's no such option."))?;
            if out.iter().any(|x| x == id) {
                return Err(bad("ranking: each option once."));
            }
            out.push(id.to_string());
        }
        out
    } else {
        let id = o.get("pick").and_then(Value::as_str).filter(|id| options.contains(id)).ok_or_else(|| bad("pick: there's no such option."))?;
        vec![id.to_string()]
    };
    if !state["ballots"].is_object() {
        state["ballots"] = json!({});
    }
    state["ballots"][who.as_str()] = json!(ballot);
    Ok(())
}

fn vote_view(spec: &Value, state: &Value, viewer: &Viewer, now: DateTime<Utc>) -> Value {
    let Some(cfg) = vote_cfg(spec) else { return Value::Null };
    let ids: Vec<&str> = collections::rows(state, cfg.collection).iter().filter_map(|r| r["id"].as_str()).collect();
    let ballots = state["ballots"].as_object();
    let voters = ballots.map_or(0, Map::len);
    let mine = match viewer {
        Viewer::Guest(id) => ballots.and_then(|b| b.get(id.as_str())).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let tally = if cfg.tally_after_reveal && !collections::revealed(spec, state, now) {
        Value::Null
    } else {
        let index = |id: &str| ids.iter().position(|x| *x == id);
        let list: Vec<Vec<usize>> = ballots.map(|b| b.values().map(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().and_then(index)).collect()).unwrap_or_default()).collect()).unwrap_or_default();
        let options: Vec<usize> = (0..ids.len()).collect();
        let t = if cfg.ranked { toolkit::count_ranked(&list, &options) } else { toolkit::count_one_pick(&list, &options) };
        let counts: Vec<Value> = t.rounds.last().map(|r| r.counts.iter().map(|(o, n)| json!({ "id": ids[*o], "votes": n })).collect()).unwrap_or_default();
        let outcome = match t.outcome {
            Outcome::Winner { option } => json!({ "kind": "winner", "id": ids[option] }),
            Outcome::Tie { options } => json!({ "kind": "tie", "ids": options.iter().map(|o| ids[*o]).collect::<Vec<_>>() }),
            Outcome::NoVotes => json!({ "kind": "no_votes" }),
        };
        json!({ "counts": counts, "outcome": outcome, "rounds": t.rounds.len() })
    };
    json!({ "mine": mine, "voters": voters, "locked": is_locked(state, &Target::Vote, now), "tally": tally })
}

// ---- draw ----

fn name_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// `apart(i, j)` over these players, from the draw config's pairs of names.
fn apart_fn<'a>(spec: &'a Value, players: &'a [&'a Player]) -> impl Fn(usize, usize) -> bool + 'a {
    let pairs: Vec<(String, String)> = config(spec, "draw")
        .and_then(|d| d["apart"].as_array())
        .map(|a| a.iter().filter_map(|p| Some((name_key(p.get(0)?.as_str()?), name_key(p.get(1)?.as_str()?)))).collect())
        .unwrap_or_default();
    move |i: usize, j: usize| {
        let (a, b) = (name_key(&players[i].name), name_key(&players[j].name));
        pairs.iter().any(|(x, y)| (*x == a && *y == b) || (*x == b && *y == a))
    }
}

fn drawn(state: &Value) -> bool {
    state["draw"]["order"].is_array()
}

/// `draw {}`: each player gets one other player, in one loop (toolkit `draw_loop`), so nobody
/// draws themselves and the config's `apart` pairs never draw each other. Once.
pub fn draw(ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
    only(body, &[], "draw")?;
    if drawn(state) {
        return Err(not_now("The names are drawn already."));
    }
    let players: Vec<&Player> = ctx.players.iter().collect();
    let apart = apart_fn(ctx.spec, &players);
    let order = toolkit::draw_loop(players.len(), &apart, &mut Rng::from_os()).map_err(|b| match b {
        Block::TooFew => not_now(format!("A draw needs at least {} players.", toolkit::MIN_PEOPLE)),
        Block::NoOneLeft(i) => not_now(format!("{} is kept apart from almost everyone, so there's nobody left to draw.", players[i].name)),
        Block::NoLoop => not_now("The pairs kept apart leave no way to draw everyone."),
    })?;
    let ids: Vec<&str> = order.iter().map(|i| players[*i].id.as_str()).collect();
    state["draw"] = json!({ "order": ids, "left_out": [] });
    Ok(())
}

/// A player who joins after the draw goes into the loop between one giver and their receiver
/// (toolkit `insert_into_loop`), so one other player's match changes; with no such spot they
/// are left out and see no match.
pub fn on_join(ctx: &KitCtx, state: &mut Value, player: &Player) {
    if !drawn(state) {
        return;
    }
    let order: Vec<String> = state["draw"]["order"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if order.contains(&player.id) {
        return;
    }
    // The players in the loop, then the new one, by index
    let mut people: Vec<&Player> = order.iter().filter_map(|id| ctx.player(id)).collect();
    // Someone in the loop is gone (they can't be removed after the draw, so only a damaged state)
    let inserted = if people.len() == order.len() {
        people.push(player);
        let apart = apart_fn(ctx.spec, &people);
        let idx: Vec<usize> = (0..order.len()).collect();
        toolkit::insert_into_loop(&idx, order.len(), &apart, &mut Rng::from_os())
    } else {
        None
    };
    match inserted {
        Some(new) => state["draw"]["order"] = json!(new.iter().map(|i| people[*i].id.as_str()).collect::<Vec<_>>()),
        None => {
            if !state["draw"]["left_out"].is_array() {
                state["draw"]["left_out"] = json!([]);
            }
            state["draw"]["left_out"].as_array_mut().expect("a list").push(json!(player.id));
        }
    }
}

/// After the draw, removing a player would change another player's match.
pub fn may_remove(state: &Value) -> Result<(), KitError> {
    if drawn(state) {
        return Err(not_now("The names are drawn, so removing someone would change another player's match."));
    }
    Ok(())
}

fn draw_view(state: &Value, players: &[Player], viewer: &Viewer) -> Value {
    if !drawn(state) {
        return Value::Null;
    }
    let order: Vec<&str> = state["draw"]["order"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    match viewer {
        Viewer::Guest(me) => {
            let mine = order.iter().position(|id| *id == me.as_str()).and_then(|k| players.iter().find(|p| p.id == order[(k + 1) % order.len()]));
            json!({ "done": true, "mine": mine.map(|p| json!({ "id": p.id, "name": p.name, "avatar": p.avatar })) })
        }
        _ => json!({ "done": true, "count": order.len() }),
    }
}

// ---- turns ----

fn turn_ids(state: &Value) -> Vec<String> {
    state["turn"]["order"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// `turn_next {}` moves the turn to the next player in the order (join order, or shuffled once
/// with toolkit `Rng` when the turns config says `shuffled`). `turn_skip {}` passes over the
/// player whose turn would come next. Players who left drop out; new ones join the end.
pub fn turn(ctx: &mut KitCtx, state: &mut Value, body: &Value, skip: bool) -> Result<(), KitError> {
    only(body, &[], if skip { "turn_skip" } else { "turn_next" })?;
    let started = state["turn"]["order"].is_array();
    let mut order = turn_ids(state);
    if !started {
        order = ctx.players.iter().map(|p| p.id.clone()).collect();
        if config(ctx.spec, "turns").and_then(|t| t["order"].as_str()) == Some("shuffled") {
            Rng::from_os().shuffle(&mut order);
        }
    }
    let current = state["turn"]["current"].as_str().map(str::to_string);
    let at = state["turn"]["at"].as_u64().unwrap_or(0) as usize;
    order.retain(|id| ctx.player(id).is_some());
    for p in ctx.players {
        if !order.contains(&p.id) {
            order.push(p.id.clone());
        }
    }
    if order.is_empty() {
        return Err(not_now("Nobody has joined yet."));
    }
    let n = order.len();
    // From the current player; if they left, the one who slid into their place is next
    let base: isize = match current.as_deref().and_then(|c| order.iter().position(|x| x == c)) {
        Some(i) => i as isize,
        None if started => at as isize - 1,
        None => -1,
    };
    let step = if skip { 2 } else { 1 };
    let raw = base + step;
    let next = raw.rem_euclid(n as isize) as usize;
    let skipped = skip.then(|| order[(base + 1).rem_euclid(n as isize) as usize].clone());
    let round = state["turn"]["round"].as_u64().unwrap_or(0) + u64::from(!started || raw >= n as isize);
    state["turn"] = json!({ "order": order, "current": order[next], "at": next, "round": round, "skipped": skipped });
    Ok(())
}

fn turn_view(state: &Value, players: &[Player]) -> Value {
    if !state["turn"]["order"].is_array() {
        return Value::Null;
    }
    let who = |id: Option<&str>| id.and_then(|id| players.iter().find(|p| p.id == id)).map(|p| json!({ "id": p.id, "name": p.name }));
    let order: Vec<String> = turn_ids(state).into_iter().filter(|id| players.iter().any(|p| p.id == *id)).collect();
    let current = state["turn"]["current"].as_str();
    let next = current.and_then(|c| order.iter().position(|x| x == c)).map(|i| order[(i + 1) % order.len()].as_str());
    json!({ "current": who(current), "next": who(next), "round": state["turn"]["round"], "skipped": who(state["turn"]["skipped"].as_str()) })
}

// ---- reveal ----

/// Publishes these groups (and with `full`, flips the `after_reveal` collections). The answer
/// collection is locked first, so no answer comes after its secret is out. `false` when nothing
/// changed.
fn do_reveal(spec: &Value, state: &mut Value, groups: &[String], full: bool) -> bool {
    let mut changed = false;
    if full && !state["revealed"].as_bool().unwrap_or(false) {
        state["revealed"] = json!(true);
        changed = true;
    }
    if !state["secrets_shown"].is_array() {
        state["secrets_shown"] = json!([]);
    }
    for g in groups {
        if let Some(a) = answer_cfg(spec).filter(|a| a.group == g) {
            if !collections::locked(state, a.collection) {
                set_lock(state, &Target::Coll(a.collection.to_string()), true);
                changed = true;
            }
        }
        let list = state["secrets_shown"].as_array_mut().expect("a list");
        if !list.iter().any(|x| x.as_str() == Some(g)) {
            list.push(json!(g));
            changed = true;
        }
    }
    changed
}

fn known_groups(spec: &Value, list: &[Value], what: &str) -> Result<Vec<String>, KitError> {
    let mut out = Vec::new();
    for v in list {
        let g = v.as_str().filter(|g| spec["secrets"].get(*g).is_some()).ok_or_else(|| bad(format!("{what}: there's no secret group called {:?}.", short(&v.to_string()))))?;
        out.push(g.to_string());
    }
    Ok(out)
}

/// `reveal {}`: the full reveal (the `after_reveal` collections and the reveal config's secret
/// groups). `reveal {secrets: [..]}`: only those groups.
pub fn reveal(ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
    let o = only(body, &["secrets"], "reveal")?;
    let changed = match o.get("secrets") {
        None | Some(Value::Null) => do_reveal(ctx.spec, state, &full_reveal_groups(ctx.spec), true),
        Some(Value::Array(a)) if !a.is_empty() => {
            let groups = known_groups(ctx.spec, a, "secrets")?;
            do_reveal(ctx.spec, state, &groups, false)
        }
        _ => return Err(bad("secrets: send a list of secret group names.")),
    };
    if !changed {
        return Err(not_now("That's revealed already."));
    }
    Ok(())
}

// ---- steps ----

/// `step {}`: the next `steps[]` entry. `open` unlocks a collection (with `timer`, for that many
/// seconds), `lock` locks one, `reveal` publishes the named groups (`true`: the full reveal),
/// `say` is a line every screen shows.
pub fn step(ctx: &mut KitCtx, state: &mut Value, body: &Value) -> Result<(), KitError> {
    only(body, &[], "step")?;
    let steps = ctx.spec["steps"].as_array().map_or(&[][..], Vec::as_slice);
    if steps.is_empty() {
        return Err(not_now("This app has no steps."));
    }
    let i = state["step"]["at"].as_u64().map_or(0, |n| n as usize + 1);
    let Some(entry) = steps.get(i) else {
        return Err(not_now("That was the last step."));
    };
    if let Some(c) = entry["open"].as_str() {
        let t = Target::Coll(c.to_string());
        set_lock(state, &t, false);
        if let Some(secs) = entry["timer"].as_u64() {
            start_timer(ctx, state, &t, secs.clamp(TIMER_MIN, TIMER_MAX));
        }
    }
    if let Some(c) = entry["lock"].as_str() {
        let t = Target::Coll(c.to_string());
        set_lock(state, &t, true);
        if timer_end(state).is_some_and(|(tt, _)| tt == t) {
            state["timer"] = Value::Null;
        }
    }
    match &entry["reveal"] {
        Value::Bool(true) => {
            do_reveal(ctx.spec, state, &full_reveal_groups(ctx.spec), true);
        }
        Value::Array(a) => {
            let groups: Vec<String> = a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect();
            do_reveal(ctx.spec, state, &groups, false);
        }
        _ => {}
    }
    state["step"] = json!({ "at": i });
    Ok(())
}

fn step_view(spec: &Value, state: &Value) -> Value {
    let steps = spec["steps"].as_array().map_or(&[][..], Vec::as_slice);
    let Some(at) = state["step"]["at"].as_u64().map(|n| n as usize).filter(|n| *n < steps.len()) else {
        return if steps.is_empty() { Value::Null } else { json!({ "at": null, "of": steps.len(), "say": null }) };
    };
    let say = steps[at]["say"].as_str().and_then(|s| words::clean_text(s, SAY_MAX).ok());
    json!({ "at": at, "of": steps.len(), "say": say })
}

// ---- tick ----

/// A timer that ran out locks its collection (or the vote); `reveal_at` passing is the full
/// reveal. `true` when the state changed.
pub fn tick(ctx: &KitCtx, state: &mut Value) -> bool {
    let mut changed = false;
    if let Some((t, end)) = timer_end(state) {
        if end <= ctx.now {
            set_lock(state, &t, true);
            state["timer"] = Value::Null;
            changed = true;
        }
    }
    if reveal_at_passed(ctx.spec, ctx.now) {
        changed |= do_reveal(ctx.spec, state, &full_reveal_groups(ctx.spec), true);
    }
    changed
}

// ---- views ----

/// The parts of one viewer's view the actions add: `revealed`, `step`, `timer`, `turn`,
/// `scores`, `vote` and `draw`.
pub fn view_parts(spec: &Value, state: &Value, players: &[Player], viewer: &Viewer, now: DateTime<Utc>) -> Map<String, Value> {
    let mut revealed = Map::new();
    for g in shown(spec, state, now) {
        let texts: Vec<Value> = group(spec, &g).into_iter().map(|s| json!(s.text)).collect();
        revealed.insert(g, Value::Array(texts));
    }
    let timer = match timer_end(state) {
        Some((t, end)) if end > now => {
            let mut v = target_json(&t);
            v["ends_at"] = state["timer"]["ends_at"].clone();
            v["seconds_left"] = json!((end - now).num_seconds().max(0));
            v
        }
        _ => Value::Null,
    };
    let mut out = Map::new();
    out.insert("revealed".into(), Value::Object(revealed));
    out.insert("step".into(), step_view(spec, state));
    out.insert("timer".into(), timer);
    out.insert("turn".into(), turn_view(state, players));
    out.insert("scores".into(), scores(spec, state, players, now).unwrap_or(Value::Null));
    out.insert("vote".into(), vote_view(spec, state, viewer, now));
    out.insert("draw".into(), draw_view(state, players, viewer));
    out
}

/// A removed player's ballot goes with them (their rows go in [`collections::forget`]).
pub fn forget(state: &mut Value, player_id: &str) {
    if let Some(b) = state["ballots"].as_object_mut() {
        b.remove(player_id);
    }
}

// ---- the spec's `secrets`, `actions` and `steps` ----

/// Checks `secrets`, `actions` and `steps` (the collections are checked already). `Err` is one
/// plain sentence.
pub fn check_spec(spec: &Value) -> Result<(), String> {
    check_secrets(&spec["secrets"])?;
    if let Some(a) = spec["actions"].as_object() {
        if let Some(k) = a.keys().find(|k| !CONFIGS.contains(&k.as_str())) {
            return Err(format!("actions has {:?}, which isn't an action. Known: {}.", short(k), CONFIGS.join(", ")));
        }
        for (k, v) in a {
            if !v.is_object() {
                return Err(format!("actions.{k} must be an object."));
            }
        }
    }
    check_answer(spec)?;
    check_vote(spec)?;
    check_draw(spec)?;
    check_simple(spec)?;
    check_steps(spec)
}

fn keys_only(v: &Value, keys: &[&str], what: &str) -> Result<(), String> {
    if let Some(k) = v.as_object().and_then(|o| o.keys().find(|k| !keys.contains(&k.as_str()))) {
        return Err(format!("{what} doesn't take {:?}. It takes {}.", short(k), keys.join(", ")));
    }
    Ok(())
}

fn check_text(v: &Value, what: &str) -> Result<(), String> {
    let s = v.as_str().ok_or_else(|| format!("{what} must be a text."))?;
    if s.trim().is_empty() || s.chars().count() > SECRET_MAX {
        return Err(format!("{what} must be 1 to {SECRET_MAX} characters."));
    }
    Ok(())
}

fn check_item(v: &Value, what: &str) -> Result<(), String> {
    match v {
        Value::String(_) => check_text(v, what),
        Value::Object(o) => {
            keys_only(v, &["text", "accept"], what)?;
            check_text(o.get("text").unwrap_or(&Value::Null), &format!("{what}.text"))?;
            if let Some(a) = o.get("accept") {
                let a = a.as_array().filter(|a| a.len() <= MAX_ACCEPT).ok_or_else(|| format!("{what}.accept must be a list of at most {MAX_ACCEPT} texts."))?;
                for x in a {
                    check_text(x, &format!("{what}.accept"))?;
                }
            }
            Ok(())
        }
        _ => Err(format!("{what} must be a text or {{\"text\", \"accept\"}}.")),
    }
}

fn check_secrets(s: &Value) -> Result<(), String> {
    let Some(s) = s.as_object() else { return Ok(()) };
    if s.len() > MAX_GROUPS {
        return Err(format!("secrets has at most {MAX_GROUPS} groups."));
    }
    for (name, g) in s {
        if !valid_name(name) {
            return Err(format!("Secret group {:?}: a name is lower-case letters, digits and _, starting with a letter (at most 24).", short(name)));
        }
        let what = format!("secrets.{name}");
        match g {
            Value::Array(a) => {
                if a.len() > MAX_ITEMS {
                    return Err(format!("{what} has at most {MAX_ITEMS} items."));
                }
                for x in a {
                    check_item(x, &what)?;
                }
            }
            v => check_item(v, &what)?,
        }
    }
    Ok(())
}

fn coll_def<'a>(spec: &'a Value, name: &str) -> Option<&'a Value> {
    spec["collections"].get(name).filter(|v| v.is_object())
}

fn check_answer(spec: &Value) -> Result<(), String> {
    let Some(a) = config(spec, "answer") else { return Ok(()) };
    keys_only(a, &["collection", "against", "match", "points"], "actions.answer")?;
    let coll = a["collection"].as_str().ok_or("actions.answer needs collection: where the guesses go.")?;
    let def = coll_def(spec, coll).ok_or_else(|| format!("actions.answer: there's no collection called {:?}.", short(coll)))?;
    if def["write"] != "server" {
        return Err(format!("actions.answer: collection {coll:?} needs write \"server\", so guesses come only through answer."));
    }
    // A right guess is the answer: only its writer may read it back, until the reveal and after
    if def["read"] != "mine" {
        return Err(format!("actions.answer: collection {coll:?} needs read \"mine\": a right guess shown to anyone else would give the answer away."));
    }
    let fields = def["fields"].as_object().cloned().unwrap_or_default();
    if fields.get("text").and_then(|f| f["type"].as_str()) != Some("text") {
        return Err(format!("actions.answer: collection {coll:?} needs a text field called \"text\"."));
    }
    if let Some(k) = fields.keys().find(|k| *k != "text" && *k != "q") {
        return Err(format!("actions.answer: collection {coll:?} has only the fields text and q, not {:?}.", short(k)));
    }
    let against = a["against"].as_str().ok_or("actions.answer needs against: the secret group to answer, like \"secrets.answers\".")?;
    let g = group_name(against);
    if spec["secrets"].get(g).is_none() {
        return Err(format!("actions.answer: there's no secret group called {:?}.", short(g)));
    }
    let n = group(spec, g).len();
    if n > 1 {
        let q = fields.get("q").filter(|f| f["type"] == "int").ok_or_else(|| format!("actions.answer: collection {coll:?} needs an int field \"q\" (which item), since {g:?} has {n} items."))?;
        if q["min"].as_i64() != Some(0) || q["max"].as_i64() != Some(n as i64 - 1) {
            return Err(format!("actions.answer: field \"q\" needs min 0 and max {}.", n - 1));
        }
    }
    if !matches!(a.get("match"), None | Some(Value::Null)) && a["match"] != "text" {
        return Err("actions.answer: match is \"text\" (the only one so far).".into());
    }
    if let Some(p) = a.get("points").filter(|p| !p.is_null()) {
        if !p.as_u64().is_some_and(|n| (1..=MAX_POINTS).contains(&n)) {
            return Err(format!("actions.answer: points must be a whole number from 1 to {MAX_POINTS}."));
        }
    }
    Ok(())
}

fn check_vote(spec: &Value) -> Result<(), String> {
    let Some(v) = config(spec, "vote") else { return Ok(()) };
    keys_only(v, &["collection", "count", "tally"], "actions.vote")?;
    let coll = v["collection"].as_str().ok_or("actions.vote needs collection: the one whose rows are the options.")?;
    let def = coll_def(spec, coll).ok_or_else(|| format!("actions.vote: there's no collection called {:?}.", short(coll)))?;
    if def["read"] != "everyone" {
        return Err(format!("actions.vote: collection {coll:?} needs read \"everyone\", so every voter sees the options."));
    }
    if !matches!(v.get("count"), None | Some(Value::Null)) && !matches!(v["count"].as_str(), Some("one_pick" | "ranked")) {
        return Err("actions.vote: count must be one_pick or ranked.".into());
    }
    if !matches!(v.get("tally"), None | Some(Value::Null)) && !matches!(v["tally"].as_str(), Some("live" | "after_reveal")) {
        return Err("actions.vote: tally must be live or after_reveal.".into());
    }
    Ok(())
}

fn check_draw(spec: &Value) -> Result<(), String> {
    let Some(d) = config(spec, "draw") else { return Ok(()) };
    keys_only(d, &["apart"], "actions.draw")?;
    match d.get("apart") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(a)) if a.len() <= MAX_APART => {
            for p in a {
                let ok = p.as_array().is_some_and(|p| p.len() == 2 && p.iter().all(|n| n.as_str().is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 40)));
                if !ok {
                    return Err("actions.draw: each apart entry is a pair of names, like [\"Ana\", \"Ben\"].".into());
                }
            }
            Ok(())
        }
        _ => Err(format!("actions.draw: apart is a list of at most {MAX_APART} pairs of names.")),
    }
}

fn check_simple(spec: &Value) -> Result<(), String> {
    if let Some(t) = config(spec, "timer") {
        keys_only(t, &["seconds"], "actions.timer")?;
        if !t["seconds"].as_u64().is_some_and(|n| (TIMER_MIN..=TIMER_MAX).contains(&n)) {
            return Err(format!("actions.timer: seconds must be a whole number from {TIMER_MIN} to {TIMER_MAX}."));
        }
    }
    if let Some(t) = config(spec, "turns") {
        keys_only(t, &["order"], "actions.turns")?;
        if !matches!(t.get("order"), None | Some(Value::Null)) && !matches!(t["order"].as_str(), Some("join" | "shuffled")) {
            return Err("actions.turns: order must be join or shuffled.".into());
        }
    }
    if let Some(r) = config(spec, "reveal") {
        keys_only(r, &["collections", "secrets"], "actions.reveal")?;
        for c in r["collections"].as_array().map_or(&[][..], Vec::as_slice) {
            let name = c.as_str().unwrap_or("");
            if coll_def(spec, name).is_none_or(|d| d["read"] != "after_reveal") {
                return Err(format!("actions.reveal: {:?} isn't an after_reveal collection.", short(name)));
            }
        }
        for g in r["secrets"].as_array().map_or(&[][..], Vec::as_slice) {
            if g.as_str().is_none_or(|g| spec["secrets"].get(g).is_none()) {
                return Err(format!("actions.reveal: there's no secret group called {:?}.", short(&g.to_string())));
            }
        }
    }
    Ok(())
}

fn check_steps(spec: &Value) -> Result<(), String> {
    let Some(steps) = spec["steps"].as_array() else { return Ok(()) };
    if steps.len() > MAX_STEPS {
        return Err(format!("steps has at most {MAX_STEPS} entries."));
    }
    for (i, s) in steps.iter().enumerate() {
        let what = format!("Step {}", i + 1);
        let o = s.as_object().filter(|o| !o.is_empty()).ok_or_else(|| format!("{what} must be an object with one of {}.", STEP_KEYS.join(", ")))?;
        keys_only(s, STEP_KEYS, &what)?;
        if let Some(say) = o.get("say") {
            let t = say.as_str().ok_or_else(|| format!("{what}: say must be a text."))?;
            words::clean_text(t, SAY_MAX).map_err(|e| format!("{what}: say: {}", e.message()))?;
        }
        for k in ["open", "lock"] {
            if let Some(c) = o.get(k) {
                let name = c.as_str().unwrap_or("");
                if coll_def(spec, name).is_none() {
                    return Err(format!("{what}: there's no collection called {:?} to {k}.", short(name)));
                }
            }
        }
        if let Some(t) = o.get("timer") {
            if o.get("open").is_none() {
                return Err(format!("{what}: a timer goes with open (the collection it closes)."));
            }
            if !t.as_u64().is_some_and(|n| (TIMER_MIN..=TIMER_MAX).contains(&n)) {
                return Err(format!("{what}: timer must be a whole number of seconds from {TIMER_MIN} to {TIMER_MAX}."));
            }
        }
        match o.get("reveal") {
            None | Some(Value::Bool(true)) => {}
            Some(Value::Array(a)) if !a.is_empty() => {
                for g in a {
                    if g.as_str().is_none_or(|g| spec["secrets"].get(g).is_none()) {
                        return Err(format!("{what}: there's no secret group called {:?}.", short(&g.to_string())));
                    }
                }
            }
            Some(_) => return Err(format!("{what}: reveal is true or a list of secret group names.")),
        }
    }
    Ok(())
}
