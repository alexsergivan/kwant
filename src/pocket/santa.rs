//! The Secret Santa kit (pa10a). Spec and design:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa10a/ (design.md "For Lin").
//!
//! People join with a name and a 4-digit PIN (the core's PIN rules), the names are drawn into
//! one loop that keeps couples apart, and from then on **each person sees only their own match**.
//! The host (the person's card and the host remote) sees counts and per-person ticks, never a
//! match and never a question's words. Santa questions travel along one pair only: the giver
//! asks, the receiver answers, and the receiver never learns who asked. On the gift day the host
//! may show the whole loop to everyone (the reveal), or skip it.
//!
//! Everything lives in the pocket's kit state on the host's computer and is deleted with the
//! pocket (7 days after the reveal date, else the gift date). No model runs after the draw:
//! the kit is plain code. Matches are an app promise, not cryptography: whoever controls the
//! computer could read the database, and the copy never claims more than that.
//!
//! Spec (`POST /api/pockets` with `kit: "santa"`):
//! `title` (required), `gift_on` (required, `YYYY-MM-DD` or RFC 3339), `reveal` (bool, default
//! true), `reveal_at` (RFC 3339, required while `reveal` is true: the bot writes 7 PM on the gift
//! day in the person's time zone), `draw_at` (RFC 3339, optional), `expected` (3-60, optional),
//! `people` (names expected, optional), `keep_apart` (`[["Mia","Sam"], ...]`), `budget` (text,
//! for example "€25"), `host_name`.

use super::kits::{spec_date, Kit, KitCtx, KitError, KitRules, Player, Retention, Viewer, DEFAULT_MAX_PLAYERS};
use super::words;
use chrono::{DateTime, Utc};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const ID: &str = "santa";
/// Fewest people a draw takes.
pub const MIN_PEOPLE: usize = 3;
/// Longest Santa question or answer, in characters.
pub const ASK_MAX: usize = 200;
/// Longest wish list, in characters.
pub const WISH_MAX: usize = 300;
/// Questions plus answers one person may send per day (UTC).
pub const SENDS_PER_DAY: i64 = 10;
/// Kept-apart pairs per pocket.
pub const MAX_PAIRS: usize = 60;
const DATE_FIELDS: &[&str] = &["reveal_at", "gift_on"];
const RETENTION_DAYS: i64 = 7;

pub const COLLECTING: &str = "collecting";
pub const DRAWN: &str = "drawn";
pub const REVEALED: &str = "revealed";

// ---- The draw (pure, seeded in tests) ----

/// A small seedable generator (SplitMix64): tests replay draws by seed; real draws seed it from
/// the operating system.
pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn from_os() -> Self {
        let mut b = [0u8; 8];
        let _ = ring::rand::SystemRandom::new().fill(&mut b);
        Rng(u64::from_le_bytes(b))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` (n ≤ 60, so the modulo bias is far below anything a person could notice).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// Why names can't be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Fewer than [`MIN_PEOPLE`].
    TooFew,
    /// This person is kept apart from everyone but one (a loop needs a giver and a receiver).
    NoOneLeft(usize),
    /// Everyone has two people left, yet no single loop fits the pairs.
    NoLoop,
}

/// Tries before the backtracking search; a random order of 9 people with 2 couples fits about
/// half the time, so this almost never runs out.
const SHUFFLES: usize = 2000;
/// Steps the backtracking search may take before it gives up (well under a second).
const SEARCH_STEPS: u64 = 2_000_000;

/// One random loop through `n` people: `order[k]` gives to `order[k + 1]`, the last to the
/// first. `apart(a, b)` (symmetric) keeps two people from giving to each other. A single loop
/// means nobody draws themselves and, from 3 people on, no two people draw each other.
pub fn draw_loop(n: usize, apart: &dyn Fn(usize, usize) -> bool, rng: &mut Rng) -> Result<Vec<usize>, Block> {
    if n < MIN_PEOPLE {
        return Err(Block::TooFew);
    }
    let ok = |a: usize, b: usize| a != b && !apart(a, b) && !apart(b, a);
    if let Some(i) = (0..n).find(|&i| (0..n).filter(|&j| ok(i, j)).count() < 2) {
        return Err(Block::NoOneLeft(i));
    }
    // Uniform over the loops that fit: shuffle and keep the first that fits
    let mut order: Vec<usize> = (0..n).collect();
    for _ in 0..SHUFFLES {
        rng.shuffle(&mut order);
        if (0..n).all(|k| ok(order[k], order[(k + 1) % n])) {
            return Ok(order);
        }
    }
    // Tight rules: a randomised depth-first search finds a loop or shows there is none
    let start = rng.below(n);
    let mut path = vec![start];
    let mut used = vec![false; n];
    used[start] = true;
    let mut steps = SEARCH_STEPS;
    if extend(&mut path, &mut used, n, &ok, rng, &mut steps) {
        return Ok(path);
    }
    Err(Block::NoLoop)
}

fn extend(path: &mut Vec<usize>, used: &mut [bool], n: usize, ok: &dyn Fn(usize, usize) -> bool, rng: &mut Rng, steps: &mut u64) -> bool {
    let last = path[path.len() - 1];
    if path.len() == n {
        return ok(last, path[0]);
    }
    if *steps == 0 {
        return false;
    }
    *steps -= 1;
    let mut next: Vec<usize> = (0..n).filter(|&j| !used[j] && ok(last, j)).collect();
    rng.shuffle(&mut next);
    for j in next {
        used[j] = true;
        path.push(j);
        if extend(path, used, n, ok, rng, steps) {
            return true;
        }
        path.pop();
        used[j] = false;
    }
    false
}

/// A late joiner `new` goes between one giver and their receiver, so exactly one existing
/// person's match changes. `None` when the pairs leave no such spot.
pub fn insert_into_loop(order: &[usize], new: usize, apart: &dyn Fn(usize, usize) -> bool, rng: &mut Rng) -> Option<Vec<usize>> {
    let n = order.len();
    let ok = |a: usize, b: usize| a != b && !apart(a, b) && !apart(b, a);
    let spots: Vec<usize> = (0..n).filter(|&k| ok(order[k], new) && ok(new, order[(k + 1) % n])).collect();
    if spots.is_empty() {
        return None;
    }
    let k = spots[rng.below(spots.len())];
    let mut out = order.to_vec();
    out.insert(k + 1, new);
    Some(out)
}

// ---- Kit state ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct St {
    v: u32,
    phase: String,
    /// `scheduled`, `skipped` (the host's Skip; Show now still works), `never` (the spec said
    /// keep it secret forever) or `shown`.
    reveal: String,
    /// Kept-apart pairs, as the names were typed; compared by name key.
    apart: Vec<[String; 2]>,
    /// Names the host let in after the draw (name key → name as typed).
    late: BTreeMap<String, String>,
    /// Player ids: each gives to the next, the last to the first.
    ring: Vec<String>,
    drawn_at: Option<DateTime<Utc>>,
    revealed_at: Option<DateTime<Utc>>,
    /// When each player first opened their current match.
    looked: BTreeMap<String, DateTime<Utc>>,
    /// Givers who ticked "I've got the gift".
    ready: BTreeSet<String>,
    wish: BTreeMap<String, String>,
    asks: Vec<Ask>,
    /// Questions plus answers sent today: player → (UTC day, count).
    sent: BTreeMap<String, (String, i64)>,
    /// A late joiner changed this player's match (the page says so until they open it again).
    /// `looked` and `ready` stay as they were, so the host's ticks and counts never show who
    /// buys for the late joiner; only this player's own page treats the match as unopened.
    changed: BTreeMap<String, DateTime<Utc>>,
    /// Givers whose "I've got the gift" tick was for the match before a late joiner changed it.
    /// The host still sees the tick; the giver's page shows it off until they tick again.
    ready_old: BTreeSet<String>,
    /// Why Draw is off right now (only while collecting).
    block: Option<BlockInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Ask {
    id: String,
    giver: String,
    receiver: String,
    q: String,
    a: Option<String>,
    at: Option<DateTime<Utc>>,
    answered_at: Option<DateTime<Utc>>,
    /// The receiver has seen the question.
    seen: bool,
    /// The giver has seen the answer.
    answer_seen: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct BlockInfo {
    code: String,
    message: String,
}

fn load(state: &Value) -> St {
    serde_json::from_value(state.clone()).unwrap_or_default()
}

fn store(state: &mut Value, s: &St) {
    if let Ok(v) = serde_json::to_value(s) {
        *state = v;
    }
}

/// The name key the core keeps unique (lower case, NFKC).
fn key(name: &str) -> String {
    words::clean_name(name).map(|(_, k)| k).unwrap_or_else(|_| name.trim().to_lowercase())
}

fn new_id() -> String {
    let mut b = [0u8; 6];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl St {
    fn receiver_of(&self, id: &str) -> Option<&str> {
        let k = self.ring.iter().position(|p| p == id)?;
        Some(self.ring[(k + 1) % self.ring.len()].as_str())
    }

    /// Whether this player has opened their current match (a late joiner's change closes it
    /// again for them, though the host's `looked` tick stays).
    fn opened(&self, id: &str) -> bool {
        self.looked.contains_key(id) && !self.changed.contains_key(id)
    }

    fn santa_of(&self, id: &str) -> Option<&str> {
        let k = self.ring.iter().position(|p| p == id)?;
        Some(self.ring[(k + self.ring.len() - 1) % self.ring.len()].as_str())
    }

    /// `apart(i, j)` over `keys` (player name keys, in the order the draw uses).
    fn apart_fn<'a>(&'a self, keys: &'a [String]) -> impl Fn(usize, usize) -> bool + 'a {
        let pairs: Vec<(String, String)> = self.apart.iter().map(|[a, b]| (key(a), key(b))).collect();
        move |i: usize, j: usize| pairs.iter().any(|(a, b)| (keys[i] == *a && keys[j] == *b) || (keys[i] == *b && keys[j] == *a))
    }

    fn describe(&self, b: &Block, players: &[Player]) -> BlockInfo {
        let (code, message) = match b {
            Block::TooFew => ("too_few", format!("Santa needs at least {MIN_PEOPLE} people to draw names.")),
            Block::NoOneLeft(i) => (
                "no_one_left",
                format!("The kept-apart pairs leave {} with no one to buy for. Remove a pair or wait for more people.", players.get(*i).map_or("someone", |p| p.name.as_str())),
            ),
            Block::NoLoop => ("no_loop", "The kept-apart pairs leave no way to draw names. Remove a pair or wait for more people.".to_string()),
        };
        BlockInfo { code: code.into(), message }
    }

    /// Whether Draw is possible now (stored, so a view never runs the search).
    fn recheck(&mut self, players: &[Player]) {
        if self.phase != COLLECTING {
            self.block = None;
            return;
        }
        let keys: Vec<String> = players.iter().map(|p| key(&p.name)).collect();
        let res = draw_loop(players.len(), &self.apart_fn(&keys), &mut Rng::seeded(0x5A17A));
        self.block = res.err().map(|b| self.describe(&b, players));
    }

    fn draw(&mut self, ctx: &mut KitCtx, rng: &mut Rng) -> Result<(), BlockInfo> {
        let keys: Vec<String> = ctx.players.iter().map(|p| key(&p.name)).collect();
        let order = draw_loop(ctx.players.len(), &self.apart_fn(&keys), rng).map_err(|b| self.describe(&b, ctx.players))?;
        self.ring = order.into_iter().map(|i| ctx.players[i].id.clone()).collect();
        self.phase = DRAWN.into();
        self.drawn_at = Some(ctx.now);
        self.looked.clear();
        self.ready.clear();
        self.changed.clear();
        self.ready_old.clear();
        self.asks.clear();
        self.block = None;
        // Counts only: an event row never names a player or a pair
        ctx.event(None, "drawn", json!({ "people": self.ring.len() }));
        Ok(())
    }

    /// Puts a late joiner (already in `players`) into the loop: between one giver and their
    /// receiver once someone has looked, or a fresh draw while nobody has.
    fn place_late(&mut self, ctx: &mut KitCtx, new_id: &str, rng: &mut Rng) -> Result<(), String> {
        if self.looked.is_empty() {
            return self.draw(ctx, rng).map_err(|b| b.message);
        }
        let players = ctx.players;
        let keys: Vec<String> = players.iter().map(|p| key(&p.name)).collect();
        let index = |id: &str| players.iter().position(|p| p.id == id);
        let order: Vec<usize> = self.ring.iter().filter_map(|id| index(id)).collect();
        let new = index(new_id).ok_or("The new person isn't in the pocket.")?;
        let out = insert_into_loop(&order, new, &self.apart_fn(&keys), rng).ok_or("The kept-apart pairs leave no place for them in the loop.")?;
        let k = out.iter().position(|&i| i == new).unwrap_or(0);
        let giver = players[out[(k + out.len() - 1) % out.len()]].id.clone();
        self.ring = out.into_iter().map(|i| players[i].id.clone()).collect();
        // The giver buys for someone new: their page says so and they open it again. Nothing
        // the host sees changes: `looked` and `ready` stay, and the old pair's questions stay
        // (counted, never shown: both pages list only the current pair's), since a tick or a
        // question count dropping would tell the host who buys for the late joiner.
        if self.ready.contains(&giver) {
            self.ready_old.insert(giver.clone());
        }
        self.changed.insert(giver, ctx.now);
        Ok(())
    }

    /// Could `name` join late now? Checked when the host adds them and again at their join.
    fn late_fits(&self, players: &[Player], name: &str) -> Result<(), String> {
        let mut all: Vec<Player> = players.to_vec();
        all.push(Player { id: "\u{0}late".into(), name: name.to_string(), avatar: String::new(), team: None, joined_at: Utc::now() });
        let keys: Vec<String> = all.iter().map(|p| key(&p.name)).collect();
        let apart = self.apart_fn(&keys);
        let fits = if self.looked.is_empty() {
            draw_loop(all.len(), &apart, &mut Rng::seeded(0x5A17A)).is_ok()
        } else {
            let order: Vec<usize> = self.ring.iter().filter_map(|id| all.iter().position(|p| &p.id == id)).collect();
            insert_into_loop(&order, all.len() - 1, &apart, &mut Rng::seeded(0x5A17A)).is_some()
        };
        if fits {
            Ok(())
        } else {
            Err(format!("The kept-apart pairs leave no place for {name} in the loop. Remove their kept-apart name and add them again."))
        }
    }

    fn sends_today(&self, id: &str, now: DateTime<Utc>) -> i64 {
        let day = now.format("%Y-%m-%d").to_string();
        self.sent.get(id).filter(|(d, _)| *d == day).map_or(0, |(_, n)| *n)
    }

    fn count_send(&mut self, id: &str, now: DateTime<Utc>) {
        let day = now.format("%Y-%m-%d").to_string();
        let n = self.sends_today(id, now);
        self.sent.insert(id.to_string(), (day, n + 1));
    }
}

fn spec_str<'a>(spec: &'a Value, field: &str) -> Option<&'a str> {
    spec.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn reveal_at(spec: &Value) -> Option<DateTime<Utc>> {
    spec.get("reveal_at").and_then(spec_date)
}

/// The start of the gift day: midnight UTC for a plain date, else the time given.
fn gift_start(spec: &Value) -> Option<DateTime<Utc>> {
    let v = spec.get("gift_on")?;
    match v.as_str().map(str::trim).and_then(|t| chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()) {
        Some(d) => d.and_hms_opt(0, 0, 0).map(|t| t.and_utc()),
        None => spec_date(v),
    }
}

/// Dates that would lose data or spoil the surprise, checked at creation (dates can't change
/// after). Everything goes `RETENTION_DAYS` after the reveal (else the gift day), and the
/// sweeper deletes any pocket past that, so a date in the past either deletes the pocket at
/// once or reveals the whole loop as soon as names are drawn. A day of slack on each side
/// covers time zones (the server doesn't know the person's).
fn check_dates(spec: &Value, now: DateTime<Utc>) -> Result<(), String> {
    let day = chrono::Duration::days(1);
    if let Some(gift_end) = spec.get("gift_on").and_then(spec_date) {
        if gift_end + day <= now {
            return Err("gift_on is already past. Use this year's gift day.".into());
        }
    }
    if let Some(r) = reveal_at(spec) {
        if r <= now {
            return Err("reveal_at is already past, so everyone would see every match as soon as names are drawn. Pick a time on or after the gift day.".into());
        }
        if let Some(g) = gift_start(spec) {
            if r < g - day {
                return Err("reveal_at is more than a day before the gift day. The reveal is on or after the gift day (7 PM on the gift day unless the person said otherwise).".into());
            }
        }
    }
    Ok(())
}

fn not_now(m: impl Into<String>) -> KitError {
    KitError::NotNow(m.into())
}

fn bad(m: impl Into<String>) -> KitError {
    KitError::BadInput(m.into())
}

fn text(body: &Value, max: usize) -> Result<String, KitError> {
    let raw = body.get("text").and_then(Value::as_str).ok_or_else(|| bad("text is required"))?;
    words::clean_text(raw, max).map_err(|e| bad(e.message()))
}

fn pair_names(body: &Value) -> Result<[String; 2], KitError> {
    let name = |f: &str| -> Result<String, KitError> {
        let raw = body.get(f).and_then(Value::as_str).ok_or_else(|| bad(format!("{f} is required")))?;
        words::clean_name(raw).map(|(shown, _)| shown).map_err(|e| bad(e.message()))
    };
    let (a, b) = (name("a")?, name("b")?);
    if key(&a) == key(&b) {
        return Err(bad("A pair needs two different names."));
    }
    Ok([a, b])
}

pub struct Santa;

impl Santa {
    fn person(p: &Player) -> Value {
        json!({ "id": p.id, "name": p.name, "avatar": p.avatar })
    }

    /// Facts every viewer (and the public lobby) may see.
    fn facts(ctx: &KitCtx, s: &St) -> Value {
        let reveal_at = match s.reveal.as_str() {
            "scheduled" | "shown" => reveal_at(ctx.spec),
            _ => None,
        };
        json!({
            "phase": s.phase,
            "gift_on": spec_str(ctx.spec, "gift_on"),
            "budget": spec_str(ctx.spec, "budget"),
            "host_name": spec_str(ctx.spec, "host_name"),
            "draw_at": ctx.spec.get("draw_at").and_then(spec_date),
            "reveal": s.reveal,
            "reveal_at": reveal_at,
            "joined": ctx.players.len(),
            "expected": Self::expected(ctx.spec),
        })
    }

    fn expected(spec: &Value) -> Option<u64> {
        spec.get("expected").and_then(Value::as_u64).or_else(|| spec.get("people").and_then(Value::as_array).map(|a| a.len() as u64)).filter(|n| *n > 0)
    }

    fn guest_view(ctx: &KitCtx, s: &St, me: &str) -> Value {
        let mut v = Self::facts(ctx, s);
        let in_ring = s.ring.iter().any(|p| p == me);
        let opened = s.opened(me);
        let receiver = s.receiver_of(me);
        let santa = s.santa_of(me);
        // The match only after "Open it", so a peek at the network tab before then shows nothing
        let my_match = receiver.filter(|_| opened).and_then(|r| ctx.player(r)).map(|p| {
            let mut m = Self::person(p);
            m["wish"] = json!(s.wish.get(&p.id));
            m
        });
        let sent: Vec<Value> = match receiver {
            Some(r) if opened => s
                .asks
                .iter()
                .filter(|a| a.giver == me && a.receiver == r)
                .map(|a| json!({ "id": a.id, "q": a.q, "a": a.a, "at": a.at, "answered_at": a.answered_at, "new_answer": a.a.is_some() && !a.answer_seen }))
                .collect(),
            _ => Vec::new(),
        };
        // Never the asker's id or name: "🎅 Santa asks"
        let received: Vec<Value> = match santa {
            Some(g) => s
                .asks
                .iter()
                .filter(|a| a.receiver == me && a.giver == g)
                .map(|a| json!({ "id": a.id, "q": a.q, "a": a.a, "at": a.at, "answered_at": a.answered_at, "new": !a.seen }))
                .collect(),
            None => Vec::new(),
        };
        let revealed = s.phase == REVEALED;
        let reveal_loop: Option<Vec<Value>> = revealed.then(|| {
            (0..s.ring.len())
                .filter_map(|k| {
                    let g = ctx.player(&s.ring[k])?;
                    let r = ctx.player(&s.ring[(k + 1) % s.ring.len()])?;
                    Some(json!({ "giver": Self::person(g), "receiver": Self::person(r) }))
                })
                .collect()
        });
        let notices: Vec<Value> = s.changed.get(me).map(|at| json!({ "kind": "match_changed", "at": at })).into_iter().collect();
        v["people"] = json!(ctx.players.iter().map(Self::person).collect::<Vec<_>>());
        v["my_wish"] = json!(s.wish.get(me));
        v["in_draw"] = json!(in_ring);
        v["can_open"] = json!(in_ring && !opened && s.phase != COLLECTING);
        v["opened"] = json!(opened);
        v["match"] = json!(my_match);
        v["gift_ready"] = json!(s.ready.contains(me) && !s.ready_old.contains(me));
        v["notices"] = json!(notices);
        v["asks"] = json!({
            "sent": sent,
            "received": received,
            "left_today": (SENDS_PER_DAY - s.sends_today(me, ctx.now)).max(0),
            "max_chars": ASK_MAX,
            "open": s.phase == DRAWN && opened,
        });
        v["my_santa"] = json!(if revealed { santa.and_then(|g| ctx.player(g)).map(Self::person) } else { None });
        v["reveal_loop"] = json!(reveal_loop);
        v
    }

    /// The host's view: counts and per-person ticks, never a match or a question's words.
    fn host_view(ctx: &KitCtx, s: &St) -> Value {
        let mut v = Self::facts(ctx, s);
        let joined: BTreeSet<String> = ctx.players.iter().map(|p| key(&p.name)).collect();
        let waiting_for: Vec<&str> = ctx
            .spec
            .get("people")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).filter(|n| !joined.contains(&key(n))).collect())
            .unwrap_or_default();
        let people: Vec<Value> = ctx
            .players
            .iter()
            .map(|p| {
                let mut row = Self::person(p);
                row["looked"] = json!(s.looked.contains_key(&p.id));
                row["gift_ready"] = json!(s.ready.contains(&p.id));
                row
            })
            .collect();
        let overdue = s.phase == COLLECTING && ctx.spec.get("draw_at").and_then(spec_date).is_some_and(|t| t <= ctx.now);
        let needs_you = s.block.as_ref().is_some_and(|b| b.code != "too_few" || overdue);
        v["people"] = json!(people);
        v["waiting_for"] = json!(waiting_for);
        v["keep_apart"] = json!(s.apart);
        v["late_allowed"] = json!(s.late.values().collect::<Vec<_>>());
        v["drawn_at"] = json!(s.drawn_at);
        v["revealed_at"] = json!(s.revealed_at);
        v["draw"] = json!({
            "can_draw": s.phase == COLLECTING && s.block.is_none(),
            "blocked": s.block,
            "overdue": overdue,
        });
        v["needs_you"] = json!(needs_you);
        v["counts"] = Self::counts(ctx, s);
        v
    }

    fn counts(ctx: &KitCtx, s: &St) -> Value {
        json!({
            "joined": ctx.players.len(),
            "looked": s.looked.len(),
            "gifts_ready": s.ready.len(),
            "questions_asked": s.asks.len(),
            "questions_answered": s.asks.iter().filter(|a| a.a.is_some()).count(),
        })
    }
}

impl Kit for Santa {
    fn id(&self) -> &'static str {
        ID
    }

    fn rules(&self, _spec: &Value) -> KitRules {
        KitRules {
            retention: Retention::AfterDate { fields: DATE_FIELDS, days: RETENTION_DAYS },
            needs_pin: true,
            max_players: DEFAULT_MAX_PLAYERS,
            guest_actions: &["open_match", "set_wish", "gift_ready", "ask", "answer", "seen_asks"],
            host_actions: &["draw_now", "add_pair", "remove_pair", "add_late", "remove_late", "reveal_now", "skip_reveal"],
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        if !spec.is_object() {
            return Err("The spec must be a JSON object.".into());
        }
        if spec_str(spec, "title").is_none() {
            return Err("The spec needs a title.".into());
        }
        let date = |f: &str| -> Result<Option<DateTime<Utc>>, String> {
            match spec.get(f) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => spec_date(v).map(Some).ok_or_else(|| format!("{f} must be a date (YYYY-MM-DD) or an RFC 3339 time.")),
            }
        };
        let gift = date("gift_on")?.ok_or("The spec needs gift_on, the day gifts are given.")?;
        let reveal = match spec.get("reveal") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("reveal must be true or false.".into()),
        };
        let reveal_at = date("reveal_at")?;
        if reveal && reveal_at.is_none() {
            return Err("With the reveal on, the spec needs reveal_at (7 PM on the gift day in the person's time zone, unless they said otherwise).".into());
        }
        let draw_at = date("draw_at")?;
        if let (Some(d), Some(r)) = (draw_at, reveal_at) {
            if r <= d {
                return Err("reveal_at must be after draw_at.".into());
            }
        }
        if let Some(d) = draw_at {
            if d > gift {
                return Err("draw_at must be before the gift day.".into());
            }
        }
        check_dates(spec, Utc::now())?;
        if let Some(e) = spec.get("expected").filter(|v| !v.is_null()) {
            match e.as_u64() {
                Some(n) if (MIN_PEOPLE as u64..=DEFAULT_MAX_PLAYERS as u64).contains(&n) => {}
                _ => return Err(format!("expected must be a whole number from {MIN_PEOPLE} to {DEFAULT_MAX_PLAYERS}.")),
            }
        }
        if let Some(p) = spec.get("people").filter(|v| !v.is_null()) {
            let names = p.as_array().ok_or("people must be a list of names.")?;
            if names.len() > DEFAULT_MAX_PLAYERS {
                return Err(format!("people holds at most {DEFAULT_MAX_PLAYERS} names."));
            }
            for n in names {
                let n = n.as_str().ok_or("people must be a list of names.")?;
                words::clean_name(n).map_err(|e| format!("{n:?} in people: {}", e.message()))?;
            }
        }
        if let Some(k) = spec.get("keep_apart").filter(|v| !v.is_null()) {
            let pairs = k.as_array().ok_or("keep_apart must be a list of name pairs.")?;
            if pairs.len() > MAX_PAIRS {
                return Err(format!("keep_apart holds at most {MAX_PAIRS} pairs."));
            }
            for pair in pairs {
                let arr = pair.as_array().filter(|a| a.len() == 2).ok_or("Each keep_apart entry is a pair of two names.")?;
                pair_names(&json!({ "a": arr[0], "b": arr[1] })).map_err(|e| match e {
                    KitError::BadInput(m) | KitError::NotNow(m) => format!("keep_apart: {m}"),
                })?;
            }
        }
        for (f, max) in [("budget", 24), ("host_name", 40)] {
            if let Some(v) = spec.get(f).filter(|v| !v.is_null()) {
                let t = v.as_str().ok_or(format!("{f} must be text."))?;
                if t.chars().count() > max {
                    return Err(format!("{f} is at most {max} characters."));
                }
            }
        }
        Ok(())
    }

    fn init(&self, spec: &Value) -> Value {
        let apart = spec
            .get("keep_apart")
            .and_then(Value::as_array)
            .map(|pairs| pairs.iter().filter_map(|p| pair_names(&json!({ "a": p.get(0)?, "b": p.get(1)? })).ok()).collect())
            .unwrap_or_default();
        let mut s = St {
            v: 1,
            phase: COLLECTING.into(),
            reveal: if spec.get("reveal").and_then(Value::as_bool) == Some(false) { "never" } else { "scheduled" }.into(),
            apart,
            ..St::default()
        };
        s.recheck(&[]);
        serde_json::to_value(&s).unwrap_or(Value::Null)
    }

    fn admit(&self, ctx: &KitCtx, state: &Value, name_key: &str) -> Result<(), KitError> {
        let s = load(state);
        match s.phase.as_str() {
            COLLECTING => Ok(()),
            DRAWN => match s.late.get(name_key) {
                Some(name) => s.late_fits(ctx.players, name).map_err(not_now),
                None => Err(not_now("The names are already drawn. Ask the host to add you.")),
            },
            _ => Err(not_now("The reveal has happened, so this Secret Santa is over.")),
        }
    }

    fn on_join(&self, ctx: &mut KitCtx, state: &mut Value, player: &Player) {
        let mut s = load(state);
        if s.phase == COLLECTING {
            s.recheck(ctx.players);
            let reached = Self::expected(ctx.spec).is_some_and(|e| ctx.players.len() as u64 >= e);
            if reached && s.block.is_none() {
                let _ = s.draw(ctx, &mut Rng::from_os());
            }
        } else if s.late.remove(&key(&player.name)).is_some() {
            // admit() checked it fits; if the pairs changed since, Draw shows the reason
            if let Err(m) = s.place_late(ctx, &player.id, &mut Rng::from_os()) {
                s.block = Some(BlockInfo { code: "late_no_place".into(), message: m });
            }
        }
        store(state, &s);
    }

    fn may_remove(&self, _ctx: &KitCtx, state: &Value, _player_id: &str) -> Result<(), KitError> {
        if load(state).phase == COLLECTING {
            Ok(())
        } else {
            Err(not_now("After the draw, removing someone would change another person's match, so it isn't offered."))
        }
    }

    fn on_remove(&self, ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        let mut s = load(state);
        s.wish.remove(player_id);
        s.recheck(ctx.players);
        store(state, &s);
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let mut s = load(state);
        let now = ctx.now;
        match (actor, name) {
            (Viewer::Guest(me), "open_match") => {
                if s.phase == COLLECTING {
                    return Err(not_now("Names aren't drawn yet."));
                }
                if !s.ring.contains(me) {
                    return Err(not_now("You're not in the draw. Ask the host."));
                }
                s.looked.entry(me.clone()).or_insert(now);
                s.changed.remove(me);
            }
            (Viewer::Guest(me), "set_wish") => {
                if s.phase == REVEALED {
                    return Err(not_now("The gift day has passed."));
                }
                if spec_date(&ctx.spec["gift_on"]).is_some_and(|g| now > g) {
                    return Err(not_now("Wish lists close at the end of the gift day."));
                }
                let raw = body.get("text").and_then(Value::as_str).unwrap_or("");
                if raw.trim().is_empty() {
                    s.wish.remove(me);
                } else {
                    s.wish.insert(me.clone(), words::clean_text(raw, WISH_MAX).map_err(|e| bad(e.message()))?);
                }
            }
            (Viewer::Guest(me), "gift_ready") => {
                if s.phase == COLLECTING || !s.opened(me) {
                    return Err(not_now("Open your match first."));
                }
                s.ready_old.remove(me);
                if body.get("ready").and_then(Value::as_bool).unwrap_or(true) {
                    s.ready.insert(me.clone());
                } else {
                    s.ready.remove(me);
                }
            }
            (Viewer::Guest(me), "ask") => {
                if s.phase != DRAWN {
                    return Err(not_now("Santa questions are open between the draw and the reveal."));
                }
                let receiver = s.receiver_of(me).map(str::to_string).ok_or_else(|| not_now("You're not in the draw."))?;
                if !s.opened(me) {
                    return Err(not_now("Open your match first."));
                }
                if s.sends_today(me, now) >= SENDS_PER_DAY {
                    return Err(not_now(format!("That's {SENDS_PER_DAY} for today. You can send more tomorrow.")));
                }
                let q = text(body, ASK_MAX)?;
                s.count_send(me, now);
                s.asks.push(Ask { id: new_id(), giver: me.clone(), receiver, q, at: Some(now), ..Ask::default() });
            }
            (Viewer::Guest(me), "answer") => {
                if s.phase != DRAWN {
                    return Err(not_now("Santa questions are open between the draw and the reveal."));
                }
                let id = body.get("id").and_then(Value::as_str).ok_or_else(|| bad("id is required"))?;
                let santa = s.santa_of(me).map(str::to_string);
                let k = s
                    .asks
                    .iter()
                    .position(|a| a.id == id && a.receiver == *me && Some(&a.giver) == santa.as_ref())
                    .ok_or_else(|| bad("There's no such question for you."))?;
                if s.asks[k].a.is_some() {
                    return Err(not_now("You've already answered that one."));
                }
                if s.sends_today(me, now) >= SENDS_PER_DAY {
                    return Err(not_now(format!("That's {SENDS_PER_DAY} for today. You can send more tomorrow.")));
                }
                let a = text(body, ASK_MAX)?;
                s.count_send(me, now);
                let ask = &mut s.asks[k];
                ask.a = Some(a);
                ask.answered_at = Some(now);
                ask.seen = true;
            }
            (Viewer::Guest(me), "seen_asks") => {
                let (r, g) = (s.receiver_of(me).map(str::to_string), s.santa_of(me).map(str::to_string));
                for a in s.asks.iter_mut() {
                    if a.receiver == *me && Some(&a.giver) == g.as_ref() {
                        a.seen = true;
                    }
                    if a.giver == *me && Some(&a.receiver) == r.as_ref() && a.a.is_some() {
                        a.answer_seen = true;
                    }
                }
            }
            (Viewer::Host, "draw_now") => {
                if s.phase != COLLECTING {
                    return Err(not_now("The names are already drawn."));
                }
                s.draw(ctx, &mut Rng::from_os()).map_err(|b| not_now(b.message))?;
            }
            (Viewer::Host, "add_pair") => {
                if s.phase != COLLECTING {
                    return Err(not_now("Pairs can change only before the draw. To keep a late joiner apart from someone, add them with keep_apart."));
                }
                let pair = pair_names(body)?;
                let same = |p: &[String; 2]| (key(&p[0]), key(&p[1])) == (key(&pair[0]), key(&pair[1])) || (key(&p[0]), key(&p[1])) == (key(&pair[1]), key(&pair[0]));
                if !s.apart.iter().any(same) {
                    if s.apart.len() >= MAX_PAIRS {
                        return Err(bad(format!("At most {MAX_PAIRS} pairs.")));
                    }
                    s.apart.push(pair);
                }
                s.recheck(ctx.players);
            }
            (Viewer::Host, "remove_pair") => {
                if s.phase != COLLECTING {
                    return Err(not_now("Pairs can change only before the draw."));
                }
                let pair = pair_names(body)?;
                let (a, b) = (key(&pair[0]), key(&pair[1]));
                s.apart.retain(|p| !((key(&p[0]) == a && key(&p[1]) == b) || (key(&p[0]) == b && key(&p[1]) == a)));
                s.recheck(ctx.players);
            }
            (Viewer::Host, "add_late") => {
                if s.phase == COLLECTING {
                    return Err(not_now("Before the draw anyone with the link can join."));
                }
                if s.phase == REVEALED {
                    return Err(not_now("The reveal has happened."));
                }
                let raw = body.get("name").and_then(Value::as_str).ok_or_else(|| bad("name is required"))?;
                let (shown, k) = words::clean_name(raw).map_err(|e| bad(e.message()))?;
                if ctx.players.iter().any(|p| key(&p.name) == k) {
                    return Err(bad("Someone already joined with that name."));
                }
                if ctx.players.len() + s.late.len() >= DEFAULT_MAX_PLAYERS {
                    return Err(not_now("It's full."));
                }
                let mut next = s.clone();
                if let Some(other) = body.get("keep_apart").and_then(Value::as_str).filter(|o| !o.trim().is_empty()) {
                    let pair = pair_names(&json!({ "a": shown, "b": other }))?;
                    if next.apart.len() >= MAX_PAIRS {
                        return Err(bad(format!("At most {MAX_PAIRS} pairs.")));
                    }
                    next.apart.push(pair);
                }
                next.late_fits(ctx.players, &shown).map_err(not_now)?;
                next.late.insert(k, shown);
                s = next;
            }
            (Viewer::Host, "remove_late") => {
                let raw = body.get("name").and_then(Value::as_str).ok_or_else(|| bad("name is required"))?;
                s.late.remove(&key(raw));
            }
            (Viewer::Host, "reveal_now") => {
                if s.phase != DRAWN {
                    return Err(not_now(if s.phase == COLLECTING { "Names aren't drawn yet." } else { "Everyone has already seen the reveal." }));
                }
                if s.reveal == "never" {
                    return Err(not_now("This Secret Santa stays secret: the reveal was turned off when it was made."));
                }
                s.phase = REVEALED.into();
                s.reveal = "shown".into();
                s.revealed_at = Some(now);
                ctx.event(None, "revealed", json!({}));
            }
            (Viewer::Host, "skip_reveal") => {
                if s.phase == REVEALED {
                    return Err(not_now("Everyone has already seen the reveal."));
                }
                if s.reveal != "never" {
                    s.reveal = "skipped".into();
                }
            }
            _ => return Err(bad(format!("unknown action {name}"))),
        }
        store(state, &s);
        Ok(())
    }

    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let mut s = load(state);
        let now = ctx.now;
        let changed = if s.phase == COLLECTING {
            let due = ctx.spec.get("draw_at").and_then(spec_date).is_some_and(|t| t <= now);
            due && s.block.is_none() && s.draw(ctx, &mut Rng::from_os()).is_ok()
        } else if s.phase == DRAWN && s.reveal == "scheduled" && reveal_at(ctx.spec).is_some_and(|t| t <= now) {
            s.phase = REVEALED.into();
            s.reveal = "shown".into();
            s.revealed_at = Some(now);
            ctx.event(None, "revealed", json!({}));
            true
        } else {
            false
        };
        if changed {
            store(state, &s);
        }
        changed
    }

    fn ticks_unwatched(&self) -> bool {
        true
    }

    fn stable_address_only(&self) -> bool {
        true
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let s = load(state);
        match viewer {
            Viewer::Guest(me) => Self::guest_view(ctx, &s, me),
            Viewer::Host => Self::host_view(ctx, &s),
            // No big screen for Secret Santa: the public facts only
            Viewer::BigScreen => Self::facts(ctx, &s),
        }
    }

    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        Self::facts(ctx, &load(state))
    }

    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        let mut v = Self::counts(ctx, &s);
        v["phase"] = json!(s.phase);
        v["reveal"] = json!(s.reveal);
        v["needs_you"] = Self::host_view(ctx, &s)["needs_you"].clone();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// `n` people with `pairs` kept apart, as a symmetric lookup.
    fn apart_of(n: usize, pairs: &[(usize, usize)]) -> impl Fn(usize, usize) -> bool {
        let mut m = vec![vec![false; n]; n];
        for &(a, b) in pairs {
            m[a][b] = true;
            m[b][a] = true;
        }
        move |a, b| m[a][b]
    }

    fn check_loop(order: &[usize], n: usize, pairs: &[(usize, usize)]) {
        assert_eq!(order.len(), n);
        assert_eq!(order.iter().collect::<HashSet<_>>().len(), n, "everyone exactly once");
        let mut recv = vec![usize::MAX; n];
        for k in 0..n {
            recv[order[k]] = order[(k + 1) % n];
        }
        for g in 0..n {
            let r = recv[g];
            assert_ne!(g, r, "nobody draws themselves");
            assert!(!pairs.contains(&(g, r)) && !pairs.contains(&(r, g)), "{g}->{r} is a kept-apart pair");
            if n >= 3 {
                assert_ne!(recv[r], g, "no two people draw each other ({g}<->{r})");
            }
        }
        // One loop: following receivers from 0 visits everyone before coming back
        let (mut at, mut seen) = (0, 0);
        loop {
            at = recv[at];
            seen += 1;
            if at == 0 {
                break;
            }
        }
        assert_eq!(seen, n, "a single loop");
    }

    /// Is there any loop at all? (Brute force for small n: every cyclic order starting at 0.)
    fn any_loop(n: usize, apart: &dyn Fn(usize, usize) -> bool) -> bool {
        fn go(path: &mut Vec<usize>, used: &mut Vec<bool>, n: usize, apart: &dyn Fn(usize, usize) -> bool) -> bool {
            let last = *path.last().unwrap();
            if path.len() == n {
                return !apart(last, path[0]) && last != path[0];
            }
            for j in 0..n {
                if !used[j] && !apart(last, j) {
                    used[j] = true;
                    path.push(j);
                    if go(path, used, n, apart) {
                        return true;
                    }
                    path.pop();
                    used[j] = false;
                }
            }
            false
        }
        n >= 3 && go(&mut vec![0], &mut { let mut u = vec![false; n]; u[0] = true; u }, n, apart)
    }

    #[test]
    fn five_hundred_seeded_draws_per_size_keep_every_rule() {
        let mut refused = 0;
        for n in 3..=12usize {
            for seed in 0..500u64 {
                let mut rng = Rng::seeded(seed * 1000 + n as u64);
                // Up to n/2 random couples (some sets can't be drawn: those must refuse)
                let couples = rng.below(n / 2 + 1);
                let mut pairs = Vec::new();
                let mut people: Vec<usize> = (0..n).collect();
                rng.shuffle(&mut people);
                for c in 0..couples {
                    pairs.push((people[2 * c], people[2 * c + 1]));
                }
                // Every 5th set also keeps someone apart from a third person
                if seed % 5 == 0 && n >= 4 {
                    pairs.push((people[0], people[n - 1]));
                }
                let apart = apart_of(n, &pairs);
                match draw_loop(n, &apart, &mut rng) {
                    Ok(order) => check_loop(&order, n, &pairs),
                    Err(b) => {
                        refused += 1;
                        assert!(n <= 9, "n={n} pairs={pairs:?}: refused ({b:?}) though big groups always fit these rules");
                        assert!(!any_loop(n, &apart), "n={n} pairs={pairs:?}: refused ({b:?}) but a loop exists");
                    }
                }
            }
        }
        assert!(refused > 0, "some small groups with pairs can't be drawn, and the test saw them");
    }

    #[test]
    fn draws_differ_by_seed_and_cover_the_loops() {
        // 4 people, no pairs: 3! = 6 different loops; 300 draws see them all
        let apart = apart_of(4, &[]);
        let mut seen = HashSet::new();
        for seed in 0..300 {
            let order = draw_loop(4, &apart, &mut Rng::seeded(seed)).unwrap();
            let k = order.iter().position(|&i| i == 0).unwrap();
            let rotated: Vec<usize> = (0..4).map(|j| order[(k + j) % 4]).collect();
            seen.insert(rotated);
        }
        assert_eq!(seen.len(), 6);
    }

    #[test]
    fn refusals_say_why() {
        let none = apart_of(2, &[]);
        assert_eq!(draw_loop(2, &none, &mut Rng::seeded(1)), Err(Block::TooFew));
        assert_eq!(draw_loop(0, &none, &mut Rng::seeded(1)), Err(Block::TooFew));
        // 3 people and a couple: each of the couple has only the third left
        assert!(matches!(draw_loop(3, &apart_of(3, &[(0, 1)]), &mut Rng::seeded(1)), Err(Block::NoOneLeft(_))));
        // 4 people, 0 kept apart from 1 and 2: only 3 is left for 0
        assert_eq!(draw_loop(4, &apart_of(4, &[(0, 1), (0, 2)]), &mut Rng::seeded(1)), Err(Block::NoOneLeft(0)));
        // A bow tie: everyone has two people left, but 2 is the only way between the halves
        let bowtie = apart_of(5, &[(0, 3), (0, 4), (1, 3), (1, 4)]);
        assert_eq!(draw_loop(5, &bowtie, &mut Rng::seeded(1)), Err(Block::NoLoop));
        // Two couples in 4 people: exactly one loop fits
        let two = apart_of(4, &[(0, 1), (2, 3)]);
        for seed in 0..50 {
            check_loop(&draw_loop(4, &two, &mut Rng::seeded(seed)).unwrap(), 4, &[(0, 1), (2, 3)]);
        }
        // 60 people with 30 couples still draws
        let pairs: Vec<(usize, usize)> = (0..30).map(|c| (2 * c, 2 * c + 1)).collect();
        check_loop(&draw_loop(60, &apart_of(60, &pairs), &mut Rng::seeded(9)).unwrap(), 60, &pairs);
    }

    #[test]
    fn a_late_joiner_changes_exactly_one_match() {
        for seed in 0..200 {
            let mut rng = Rng::seeded(seed);
            let pairs = [(0, 1), (4, 2)];
            let apart = apart_of(5, &pairs);
            let order = draw_loop(4, &apart, &mut rng).unwrap();
            let out = insert_into_loop(&order, 4, &apart, &mut rng).unwrap();
            check_loop(&out, 5, &pairs);
            let recv = |o: &[usize], g: usize| o[(o.iter().position(|&x| x == g).unwrap() + 1) % o.len()];
            let changed: Vec<usize> = (0..4).filter(|&g| recv(&order, g) != recv(&out, g)).collect();
            assert_eq!(changed.len(), 1, "exactly one existing match changes");
            assert_eq!(recv(&out, changed[0]), 4, "and it's to the new person");
        }
        // Kept apart from everyone's neighbours: no spot
        let order = vec![0, 1, 2];
        assert_eq!(insert_into_loop(&order, 3, &apart_of(4, &[(3, 0), (3, 1)]), &mut Rng::seeded(1)), None);
    }

    fn players(names: &[&str]) -> Vec<Player> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Player { id: format!("pl_{i}"), name: n.to_string(), avatar: "fox".into(), team: None, joined_at: Utc::now() })
            .collect()
    }

    fn spec() -> Value {
        json!({ "title": "Family Secret Santa", "gift_on": "2026-12-18", "reveal_at": "2026-12-18T18:00:00Z", "budget": "€25",
                "host_name": "Oleks", "keep_apart": [["Mia", "Sam"]] })
    }

    /// The kit's state after these people joined and the names were drawn.
    fn drawn(names: &[&str]) -> (Vec<Player>, Value) {
        let spec = spec();
        let ps = players(names);
        let mut state = Santa.init(&spec);
        for i in 0..ps.len() {
            let mut ctx = KitCtx::new(&spec, &ps[..=i], Utc::now());
            Santa.on_join(&mut ctx, &mut state, &ps[i]);
        }
        let mut ctx = KitCtx::new(&spec, &ps, Utc::now());
        Santa.act(&mut ctx, &mut state, &Viewer::Host, "draw_now", &json!({})).unwrap();
        (ps, state)
    }

    /// The same state with the loop replaced (everything else kept).
    fn with_ring(state: &Value, ring: &[&str]) -> Value {
        let mut s = load(state);
        s.ring = ring.iter().map(|r| r.to_string()).collect();
        let mut v = Value::Null;
        store(&mut v, &s);
        v
    }

    #[test]
    fn the_host_view_carries_nothing_about_the_loop() {
        // Information flow: if the host's view were any function of the loop, two different
        // loops would give two different views
        let (ps, state) = drawn(&["Mia", "Sam", "Ana", "Kai", "Leo"]);
        let spec = spec();
        let ctx = KitCtx::new(&spec, &ps, Utc::now());
        let a = with_ring(&state, &["pl_0", "pl_2", "pl_1", "pl_3", "pl_4"]);
        let b = with_ring(&state, &["pl_0", "pl_3", "pl_1", "pl_4", "pl_2"]);
        assert_ne!(a, b);
        assert_eq!(Santa.view(&ctx, &a, &Viewer::Host), Santa.view(&ctx, &b, &Viewer::Host));
        assert_eq!(Santa.host_summary(&ctx, &a), Santa.host_summary(&ctx, &b));
        assert_eq!(Santa.lobby(&ctx, &a), Santa.lobby(&ctx, &b));
        assert_eq!(Santa.view(&ctx, &a, &Viewer::BigScreen), Santa.view(&ctx, &b, &Viewer::BigScreen));
        // And a guest's view depends only on their own receiver: Ana (pl_2) gives to Sam (pl_1)
        // in both loops below, while everyone else's matches differ
        let c = with_ring(&state, &["pl_0", "pl_2", "pl_1", "pl_3", "pl_4"]);
        let d = with_ring(&state, &["pl_4", "pl_2", "pl_1", "pl_0", "pl_3"]);
        let ana = Viewer::Guest("pl_2".into());
        let mut c_open = c.clone();
        let mut d_open = d.clone();
        let mut ctx2 = KitCtx::new(&spec, &ps, ctx.now);
        Santa.act(&mut ctx2, &mut c_open, &ana, "open_match", &json!({})).unwrap();
        Santa.act(&mut ctx2, &mut d_open, &ana, "open_match", &json!({})).unwrap();
        let (vc, vd) = (Santa.view(&ctx, &c_open, &ana), Santa.view(&ctx, &d_open, &ana));
        assert_eq!(vc["match"]["name"], "Sam");
        assert_eq!(vc, vd, "Ana's view doesn't change when other people's matches do");
    }

    #[test]
    fn a_guest_sees_their_match_only_after_opening_and_never_who_asked() {
        let (ps, mut state) = drawn(&["Mia", "Sam", "Ana", "Kai"]);
        let spec = spec();
        let now = Utc::now();
        let s = load(&state);
        let mia = Viewer::Guest("pl_0".into());
        let receiver = s.receiver_of("pl_0").unwrap().to_string();
        let mut ctx = KitCtx::new(&spec, &ps, now);
        let before = Santa.view(&ctx, &state, &mia);
        assert!(before["match"].is_null() && before["can_open"] == true);
        assert!(!before.to_string().contains(&format!("\"{receiver}\"")) || ps.iter().any(|p| p.id == receiver), "ids of everyone are in people, fine");
        assert_eq!(Santa.act(&mut ctx, &mut state, &mia, "ask", &json!({ "text": "Shoe size?" })), Err(not_now("Open your match first.")));
        Santa.act(&mut ctx, &mut state, &mia, "open_match", &json!({})).unwrap();
        let after = Santa.view(&ctx, &state, &mia);
        assert_eq!(after["match"]["id"], json!(receiver));
        Santa.act(&mut ctx, &mut state, &mia, "ask", &json!({ "text": "CANARY shoe size?" })).unwrap();
        let r = Viewer::Guest(receiver.clone());
        let rv = Santa.view(&ctx, &state, &r);
        assert_eq!(rv["asks"]["received"][0]["q"], "CANARY shoe size?");
        assert_eq!(rv["asks"]["received"][0]["new"], true);
        let received = rv["asks"]["received"].to_string();
        assert!(!received.contains("pl_0") && !received.contains("Mia"), "the receiver never learns who asked: {received}");
        // Nobody else sees the question
        for p in &ps {
            if p.id != "pl_0" && p.id != receiver {
                assert!(!Santa.view(&ctx, &state, &Viewer::Guest(p.id.clone())).to_string().contains("CANARY"), "{}", p.name);
            }
        }
        assert!(!Santa.view(&ctx, &state, &Viewer::Host).to_string().contains("CANARY"));
        assert_eq!(Santa.view(&ctx, &state, &Viewer::Host)["counts"]["questions_asked"], 1);
    }

    #[test]
    fn a_late_joiner_changes_nothing_the_host_sees_whoever_the_giver_is() {
        // F2 (pa10a QA): the redraw once dropped the changed giver's ticks, so the host (who
        // typed the late name) learned who buys for the late joiner. Now every placement, with
        // any giver, leaves the host's view and summary as they were.
        let spec = json!({ "title": "Office Santa", "gift_on": "2026-12-18", "reveal_at": "2026-12-18T18:00:00Z" });
        let ps = players(&["Ana", "Ben", "Cleo", "Dev", "Eli"]);
        let now = Utc::now();
        let mut state = Santa.init(&spec);
        for i in 0..ps.len() {
            Santa.on_join(&mut KitCtx::new(&spec, &ps[..=i], now), &mut state, &ps[i]);
        }
        let mut ctx = KitCtx::new(&spec, &ps, now);
        Santa.act(&mut ctx, &mut state, &Viewer::Host, "draw_now", &json!({})).unwrap();
        // Everyone looked, ticked the gift and asked; one person answered
        for p in &ps {
            let me = Viewer::Guest(p.id.clone());
            for (name, body) in [("open_match", json!({})), ("gift_ready", json!({})), ("ask", json!({ "text": "Size?" }))] {
                Santa.act(&mut ctx, &mut state, &me, name, &body).unwrap();
            }
        }
        let first = load(&state).asks[0].clone();
        Santa.act(&mut ctx, &mut state, &Viewer::Guest(first.receiver.clone()), "answer", &json!({ "id": first.id, "text": "M" })).unwrap();
        Santa.act(&mut ctx, &mut state, &Viewer::Host, "add_late", &json!({ "name": "Yan" })).unwrap();
        let mut all = ps.clone();
        all.push(Player { id: "pl_yan".into(), name: "Yan".into(), avatar: "fox".into(), team: None, joined_at: now });
        let before = load(&state);
        let mut host_seen = HashSet::new();
        let mut givers = HashSet::new();
        for _ in 0..60 {
            let mut st = state.clone();
            let mut ctx = KitCtx::new(&spec, &all, now);
            Santa.on_join(&mut ctx, &mut st, &all[5]);
            let after = load(&st);
            assert_eq!(after.ring.len(), 6, "{:?}", after.block);
            let giver = after.santa_of("pl_yan").unwrap().to_string();
            assert_eq!(after.changed.keys().collect::<Vec<_>>(), vec![&giver], "only the giver is told");
            assert_eq!((&after.looked, &after.ready, &after.asks), (&before.looked, &before.ready, &before.asks), "nothing the host counts moves");
            assert!(ctx.events.is_empty(), "no event names the giver");
            host_seen.insert((Santa.view(&ctx, &st, &Viewer::Host).to_string(), Santa.host_summary(&ctx, &st).to_string()));
            // The giver's own page: closed again, the old tick off, the old pair's question gone
            let g = Santa.view(&ctx, &st, &Viewer::Guest(giver.clone()));
            assert_eq!((g["opened"].as_bool(), g["gift_ready"].as_bool(), g["can_open"].as_bool()), (Some(false), Some(false), Some(true)), "{g}");
            assert!(g["match"].is_null() && g["asks"]["sent"] == json!([]), "{g}");
            Santa.act(&mut ctx, &mut st, &Viewer::Guest(giver.clone()), "open_match", &json!({})).unwrap();
            let g = Santa.view(&ctx, &st, &Viewer::Guest(giver.clone()));
            assert_eq!((g["match"]["id"].as_str(), g["gift_ready"].as_bool()), (Some("pl_yan"), Some(false)), "a new gift to get");
            Santa.act(&mut ctx, &mut st, &Viewer::Guest(giver.clone()), "gift_ready", &json!({})).unwrap();
            assert_eq!(Santa.view(&ctx, &st, &Viewer::Guest(giver.clone()))["gift_ready"], true);
            host_seen.insert((Santa.view(&ctx, &st, &Viewer::Host).to_string(), Santa.host_summary(&ctx, &st).to_string()));
            givers.insert(giver);
        }
        assert!(givers.len() >= 3, "the placements reached several givers: {givers:?}");
        assert_eq!(host_seen.len(), 1, "the host's view is the same whoever the giver is, and stays so as they open and tick");
    }

    #[test]
    fn dates_that_would_lose_data_or_spoil_the_reveal_are_refused() {
        // F3 (pa10a QA): the data goes 7 days after the reveal (else the gift day), so a past
        // date deleted the pocket at once or showed every match right after the draw
        let now = "2026-12-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let s = |gift: &str, reveal: Option<&str>| json!({ "title": "x", "gift_on": gift, "reveal_at": reveal });
        let err = |spec: Value| check_dates(&spec, now).unwrap_err();
        assert!(err(s("2026-11-29", None)).starts_with("gift_on is already past"));
        assert!(err(s("2025-12-18", Some("2025-12-18T18:00:00Z"))).starts_with("gift_on is already past"), "a wrong year");
        assert!(err(s("2026-12-18", Some("2026-11-30T18:00:00Z"))).starts_with("reveal_at is already past"));
        assert!(err(s("2026-12-18", Some("2026-12-01T12:00:00Z"))).starts_with("reveal_at is already past"), "now is past");
        assert!(err(s("2026-12-18", Some("2026-12-10T18:00:00Z"))).starts_with("reveal_at is more than a day before"));
        assert!(err(s("2026-12-18", Some("2026-12-16T23:59:59Z"))).starts_with("reveal_at is more than a day before"));
        for (gift, reveal) in [
            ("2026-12-18", Some("2026-12-18T18:00:00Z")),
            ("2026-12-18", Some("2026-12-17T00:00:00Z")), // a day of slack for time zones
            ("2026-12-18", Some("2027-01-06T18:00:00Z")), // after the gift day is fine
            ("2026-12-01", Some("2026-12-01T19:00:00Z")), // the gift day is today
            ("2026-11-30", None),                         // yesterday in UTC, today somewhere
        ] {
            assert_eq!(check_dates(&s(gift, reveal), now), Ok(()), "{gift} {reveal:?}");
        }
    }
}
