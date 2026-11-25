//! The gather kit (pa10b): Sign-up, RSVP and Vote on one engine, with the Club Sub Chaser.
//! Spec and design: /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa10b/
//! (design.md "For Juno (engine)" #1-#8, server-brief.md).
//!
//! One engine, three faces (`face` in the spec):
//! - `rsvp`: Yes / Maybe / No, with plus-ones and food needs when the host chose them, and the
//!   host's own "paid" ticks when the spec names a `price`.
//! - `signup`: slots with a cap each; a full slot keeps a first-come waiting list, and when
//!   someone leaves, the first person waiting moves in at once and on their own.
//! - `vote`: one pick (plurality) or ranked (instant runoff), as a **secret ballot**: who voted
//!   is kept apart from the ballots, and no view, event row or log line pairs a person with a
//!   ballot. The host sees who voted (to nudge) and the totals, never anyone's order.
//!
//! Host-only: paid ticks, each person's food needs and plus-ones. Guests get names and
//! Yes / Maybe / No only when `show_names` is on, their own record, and their own paid tick once
//! it's ticked. The Chaser drafts nudges from templates the bot wrote when the pocket was made
//! (no model runs afterwards); nothing here ever sends anything: the host copies a draft and
//! sends it from their own chat, at most once per person every 48 hours.
//!
//! Everything lives in the pocket's kit state on the host's computer and is deleted 7 days after
//! the event (else the closing time). Like Secret Santa, the secret ballot is an app promise, not
//! cryptography: whoever controls the computer could read the database.

use super::kits::{spec_date, Kit, KitCtx, KitError, KitRules, Player, Retention, Viewer, DEFAULT_MAX_PLAYERS};
use super::words;
use chrono::{DateTime, Duration, Utc};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const ID: &str = "gather";

pub const RSVP: &str = "rsvp";
pub const SIGNUP: &str = "signup";
pub const VOTE: &str = "vote";

pub const OPEN: &str = "open";
pub const TIE: &str = "tie";
pub const CLOSED: &str = "closed";

pub const ONE_PICK: &str = "one_pick";
pub const RANKED: &str = "ranked";

/// One nudge per person in this window (design §2).
pub const NUDGE_HOURS: i64 = 48;
/// A tie's "Vote again" runs this long.
pub const REVOTE_HOURS: i64 = 24;
pub const MAX_PLUS_ONES: u64 = 5;
/// Free text: "Something else" food need, a sign-up note.
pub const NOTE_MAX: usize = 40;
pub const MAX_SLOTS: usize = 20;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 10;
pub const TEMPLATE_MAX: usize = 300;
/// Food needs a guest can tick (plus one "Something else" text).
pub const FOOD: &[&str] = &["vegetarian", "vegan", "gluten_free", "dairy_free", "nut_allergy", "halal", "kosher"];
/// The Chaser's draft kinds (and the spec's `templates` keys).
pub const NUDGES: &[&str] = &["maybe", "not_paid", "moved_up", "not_answered"];
/// Where the card puts the join link it already shows (the kit doesn't know the address).
pub const LINK: &str = "{link}";

const DATE_FIELDS: &[&str] = &["event_at", "closes_at"];
const RETENTION_DAYS: i64 = 7;
const SPEC_KEYS: &[&str] = &[
    "face", "title", "when", "place", "host_name", "price", "closes_at", "event_at", "fields", "slots", "ballot", "members", "show_names", "show_live_count",
    "waiting_list", "templates",
];

// ---- Counting (pure) ----

/// How a count ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Winner { option: usize },
    /// A final tie between these options: the host picks, or everyone votes again for 24 h.
    Tie { options: Vec<usize> },
    NoVotes,
}

/// One round of the count, for "How it was counted".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    /// `(option, votes)` for every option still in, most votes first (then option order).
    pub counts: Vec<(usize, u32)>,
    /// Ballots with no option left in this round.
    pub exhausted: u32,
    /// Options dropped after this round.
    pub dropped: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub outcome: Outcome,
    pub rounds: Vec<Round>,
}

fn round_counts(ballots: &[Vec<usize>], inn: &BTreeSet<usize>) -> (BTreeMap<usize, u32>, u32) {
    let mut counts: BTreeMap<usize, u32> = inn.iter().map(|o| (*o, 0)).collect();
    let mut exhausted = 0;
    for b in ballots {
        match b.iter().find(|o| inn.contains(o)) {
            Some(o) => *counts.entry(*o).or_default() += 1,
            None => exhausted += 1,
        }
    }
    (counts, exhausted)
}

fn sorted(counts: &BTreeMap<usize, u32>) -> Vec<(usize, u32)> {
    let mut v: Vec<(usize, u32)> = counts.iter().map(|(o, c)| (*o, *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Instant runoff (design "For Juno" #3). Count 1st choices; if no option has more than half
/// of the ballots still active, drop the lowest and move those ballots to their next ranked
/// option still in; repeat. Ballots with no ranked option left are exhausted. Ties for last
/// place: drop the one with fewer 1st choices (round 1); if still tied, drop them all, unless
/// that would drop every option still in: that's a final tie between them.
///
/// Dropping them all can leave one option; it then holds every active ballot and wins the next
/// round. (The design's "if that leaves ≥ 2 options" covers the two-way final; with one leader
/// above two tied trailers, the leader wins rather than a tie between the trailers.)
///
/// `options` are the options in play; a ranking's other entries are skipped.
pub fn count_ranked(ballots: &[Vec<usize>], options: &[usize]) -> Tally {
    let mut inn: BTreeSet<usize> = options.iter().copied().collect();
    let ballots: Vec<Vec<usize>> = ballots.iter().map(|b| b.iter().copied().filter(|o| inn.contains(o)).collect::<Vec<_>>()).filter(|b| !b.is_empty()).collect();
    let mut rounds = Vec::new();
    if ballots.is_empty() || inn.is_empty() {
        return Tally { outcome: Outcome::NoVotes, rounds };
    }
    let (first, _) = round_counts(&ballots, &inn);
    loop {
        let (counts, exhausted) = round_counts(&ballots, &inn);
        let active: u32 = counts.values().sum();
        if let Some((o, _)) = counts.iter().find(|(_, c)| 2 * **c > active) {
            rounds.push(Round { counts: sorted(&counts), exhausted, dropped: vec![] });
            return Tally { outcome: Outcome::Winner { option: *o }, rounds };
        }
        let low = counts.values().copied().min().unwrap_or(0);
        let tied: Vec<usize> = counts.iter().filter(|(_, c)| **c == low).map(|(o, _)| *o).collect();
        let fewest_first = tied.iter().map(|o| first.get(o).copied().unwrap_or(0)).min().unwrap_or(0);
        let drop: Vec<usize> = tied.into_iter().filter(|o| first.get(o).copied().unwrap_or(0) == fewest_first).collect();
        if drop.len() >= inn.len() {
            rounds.push(Round { counts: sorted(&counts), exhausted, dropped: vec![] });
            return Tally { outcome: Outcome::Tie { options: inn.into_iter().collect() }, rounds };
        }
        for o in &drop {
            inn.remove(o);
        }
        rounds.push(Round { counts: sorted(&counts), exhausted, dropped: drop });
    }
}

/// One pick: most votes wins; equal leaders are a final tie (the same rule: their 1st choices
/// are their votes, so nothing breaks it).
pub fn count_one_pick(ballots: &[Vec<usize>], options: &[usize]) -> Tally {
    let inn: BTreeSet<usize> = options.iter().copied().collect();
    let picks: Vec<Vec<usize>> = ballots.iter().filter_map(|b| b.iter().copied().find(|o| inn.contains(o))).map(|o| vec![o]).collect();
    if picks.is_empty() || inn.is_empty() {
        return Tally { outcome: Outcome::NoVotes, rounds: vec![] };
    }
    let (counts, exhausted) = round_counts(&picks, &inn);
    let top = counts.values().copied().max().unwrap_or(0);
    let leaders: Vec<usize> = counts.iter().filter(|(_, c)| **c == top).map(|(o, _)| *o).collect();
    let outcome = if leaders.len() == 1 { Outcome::Winner { option: leaders[0] } } else { Outcome::Tie { options: leaders } };
    Tally { outcome, rounds: vec![Round { counts: sorted(&counts), exhausted, dropped: vec![] }] }
}

// ---- Kit state ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct St {
    v: u32,
    /// `open`, `tie` (a vote's final tie, waiting for the host) or `closed`.
    phase: String,
    /// A "vote again" moved the closing time (else the spec's `closes_at`).
    closes_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    /// Final: the core ends the pocket (a vote once decided; an RSVP or sign-up once the event,
    /// else the closing time, has passed, so paid ticks work until the day).
    done: bool,
    // RSVP
    answers: BTreeMap<String, Answer>,
    /// Host-only: player id → when the host ticked "paid".
    paid: BTreeMap<String, DateTime<Utc>>,
    // Sign-up
    slots: Vec<Slot>,
    /// Who moved up off a waiting list (player id → where and when): their page shows "You're
    /// in now" and the card drafts a note.
    moved: BTreeMap<String, Moved>,
    // Vote. Who voted and the ballots are kept apart: `voted` has people and times, `ballots`
    // has orders under a key that is not a player id (salted hash), in key order, not time order.
    ballot_salt: String,
    voted: BTreeMap<String, DateTime<Utc>>,
    ballots: BTreeMap<String, Vec<usize>>,
    /// Options still in after a tie's "vote again" (empty: all).
    only: Vec<usize>,
    tally: Option<Tally>,
    /// The host picked the winner of a tie.
    picked: bool,
    // Chaser: name key → when the host last copied a note for that person.
    copied: BTreeMap<String, DateTime<Utc>>,
    /// pa10c: the host's "Delete answers now" (or "Delete ballots now"): when, and the counts
    /// kept in place of everyone's answers. The vote's `tally` stays as it is (option counts).
    deleted: Option<Kept>,
}

/// What stays after "Delete answers now": counts and the summary line, never a name, a note,
/// a food need, a paid tick or a ballot (design rule 8).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Kept {
    at: Option<DateTime<Utc>>,
    summary: String,
    joined: usize,
    answered: usize,
    /// Listed members who never answered (or voted): the card's "N never answered".
    never: usize,
    /// RSVP: yes / maybe / no / plus_ones / coming.
    rsvp: Value,
    /// Sign-up: (filled, waiting) per slot.
    slots: Vec<(usize, usize)>,
    /// Vote: ballots counted (one per voter).
    ballots: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Answer {
    answer: String,
    plus_ones: u64,
    food: Vec<String>,
    food_other: Option<String>,
    at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Slot {
    taken: Vec<Taken>,
    /// Player ids, first in line first.
    waiting: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Taken {
    id: String,
    note: Option<String>,
    at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Moved {
    slot: usize,
    at: Option<DateTime<Utc>>,
    seen: bool,
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

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    let _ = ring::rand::SystemRandom::new().fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn not_now(m: impl Into<String>) -> KitError {
    KitError::NotNow(m.into())
}

fn bad(m: impl Into<String>) -> KitError {
    KitError::BadInput(m.into())
}

fn spec_str<'a>(spec: &'a Value, field: &str) -> Option<&'a str> {
    spec.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn face(spec: &Value) -> &str {
    spec_str(spec, "face").unwrap_or("")
}

fn flag(spec: &Value, field: &str) -> bool {
    spec.get(field).and_then(Value::as_bool).unwrap_or(true)
}

/// The fields the host chose beyond the face's own (plus_ones, food_needs, slot_note).
fn has_field(spec: &Value, field: &str) -> bool {
    spec.get("fields").and_then(Value::as_array).is_some_and(|a| a.iter().any(|f| f.as_str() == Some(field)))
}

fn tracks_paid(spec: &Value) -> bool {
    face(spec) == RSVP && spec_str(spec, "price").is_some()
}

fn slot_specs(spec: &Value) -> Vec<(String, usize)> {
    spec.get("slots")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|s| (spec_str(s, "name").unwrap_or("").to_string(), s.get("cap").and_then(Value::as_u64).unwrap_or(1) as usize)).collect())
        .unwrap_or_default()
}

fn options(spec: &Value) -> Vec<String> {
    spec.pointer("/ballot/options").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

fn ballot_kind(spec: &Value) -> &str {
    spec.pointer("/ballot/kind").and_then(Value::as_str).unwrap_or(ONE_PICK)
}

/// Members the host listed (as typed), deduplicated by name key.
fn members(spec: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    spec.get("members")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter_map(|n| words::clean_name(n).ok()).filter(|(_, k)| seen.insert(k.clone())).map(|(shown, _)| shown).collect())
        .unwrap_or_default()
}

fn template<'a>(spec: &'a Value, kind: &str) -> Option<&'a str> {
    spec.get("templates").and_then(|t| t.get(kind)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn default_template(face: &str, kind: &str) -> &'static str {
    match (face, kind) {
        (_, "maybe") => "Hi {name}! Still a maybe for {title}? Tap to change your answer: {link}",
        (_, "not_paid") => "Hi {name}, a quick one: your share for {title} is still open. Pay {host} whenever suits you before the day. Thanks!",
        (_, "moved_up") => "Hi {name}, good news: a spot opened in {slot} for {title}, so you're in now. If you can't make it, let it go here: {link}",
        (VOTE, _) => "Hi {name}! There's a vote: {title}. It takes a few seconds: {link}",
        (SIGNUP, _) => "Hi {name}! There's a sign-up: {title}. Pick what you'd like to bring: {link}",
        _ => "Hi {name}! Could you answer for {title}? It takes a few seconds: {link}",
    }
}

impl St {
    /// When answers close now (a "vote again" moves it).
    fn closes(&self, spec: &Value) -> Option<DateTime<Utc>> {
        self.closes_at.or_else(|| spec.get("closes_at").and_then(spec_date))
    }

    /// pa10c: keeps the counts and the summary, then forgets every answer, name key, note,
    /// food need, paid tick, waiting list, moved-up note, nudge time and ballot. Again: nothing
    /// left to forget, and the first time stays.
    fn delete_answers(&mut self, ctx: &KitCtx) {
        if self.deleted.is_none() {
            self.deleted = Some(Kept {
                at: Some(ctx.now),
                summary: Gather::summary_line(ctx, self),
                joined: ctx.players.len(),
                answered: self.answered_count(ctx),
                never: self.not_answered(ctx).len(),
                rsvp: Gather::rsvp_counts(self),
                slots: self.slots.iter().map(|x| (x.taken.len(), x.waiting.len())).collect(),
                ballots: self.ballots.len(),
            });
        }
        self.answers.clear();
        self.paid.clear();
        for slot in self.slots.iter_mut() {
            slot.taken.clear();
            slot.waiting.clear();
        }
        self.moved.clear();
        self.voted.clear();
        self.ballots.clear();
        self.ballot_salt.clear();
        self.copied.clear();
    }

    /// People who joined (kept after a delete: the players are gone then).
    fn joined(&self, ctx: &KitCtx) -> usize {
        self.deleted.as_ref().map_or(ctx.players.len(), |k| k.joined)
    }

    /// How many listed members never answered (or voted), kept after a delete (the names go).
    fn not_answered_count(&self, ctx: &KitCtx) -> usize {
        self.deleted.as_ref().map_or_else(|| self.not_answered(ctx).len(), |k| k.never)
    }

    /// A slot's (filled, waiting), kept after a delete.
    fn slot_counts(&self, i: usize) -> (usize, usize) {
        match &self.deleted {
            Some(k) => k.slots.get(i).copied().unwrap_or_default(),
            None => self.slots.get(i).map_or((0, 0), |x| (x.taken.len(), x.waiting.len())),
        }
    }

    /// How many voted (= ballots counted), kept after a delete.
    fn ballots_counted(&self) -> usize {
        self.deleted.as_ref().map_or(self.ballots.len(), |k| k.ballots)
    }

    /// Open and not past the closing time (the tick may not have run yet).
    fn taking_answers(&self, spec: &Value, now: DateTime<Utc>) -> bool {
        self.phase == OPEN && self.closes(spec).is_none_or(|c| now < c)
    }

    fn ballot_key(&self, player_id: &str) -> String {
        let d = ring::digest::digest(&ring::digest::SHA256, format!("{}:{player_id}", self.ballot_salt).as_bytes());
        d.as_ref()[..12].iter().map(|x| format!("{x:02x}")).collect()
    }

    /// The options a ballot may name now.
    fn in_play(&self, spec: &Value) -> Vec<usize> {
        if self.only.is_empty() {
            (0..options(spec).len()).collect()
        } else {
            self.only.clone()
        }
    }

    fn count(&self, spec: &Value) -> Tally {
        let ballots: Vec<Vec<usize>> = self.ballots.values().cloned().collect();
        let opts = self.in_play(spec);
        if ballot_kind(spec) == RANKED {
            count_ranked(&ballots, &opts)
        } else {
            count_one_pick(&ballots, &opts)
        }
    }

    /// 1st-choice totals per option in play (live bars).
    fn first_choices(&self, spec: &Value) -> Vec<Value> {
        let opts = self.in_play(spec);
        let inn: BTreeSet<usize> = opts.iter().copied().collect();
        let ballots: Vec<Vec<usize>> = self.ballots.values().cloned().collect();
        let (counts, _) = round_counts(&ballots, &inn);
        opts.iter().map(|o| json!({ "option": o, "votes": counts.get(o).copied().unwrap_or(0) })).collect()
    }

    fn slot_of(&self, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.taken.iter().any(|t| t.id == id))
    }

    fn waiting_of(&self, id: &str) -> Option<(usize, usize)> {
        self.slots.iter().enumerate().find_map(|(i, s)| s.waiting.iter().position(|w| w == id).map(|p| (i, p)))
    }

    /// Takes `id` out of their slot or waiting list; a freed place goes to the first waiting
    /// person at once (they "moved up").
    fn leave(&mut self, ctx: &mut KitCtx, id: &str, caps: &[(String, usize)]) -> bool {
        self.moved.remove(id);
        if let Some((i, p)) = self.waiting_of(id) {
            self.slots[i].waiting.remove(p);
            return true;
        }
        let Some(i) = self.slot_of(id) else { return false };
        self.slots[i].taken.retain(|t| t.id != id);
        let cap = caps.get(i).map_or(0, |c| c.1);
        while self.slots[i].taken.len() < cap && !self.slots[i].waiting.is_empty() {
            let next = self.slots[i].waiting.remove(0);
            self.slots[i].taken.push(Taken { id: next.clone(), note: None, at: Some(ctx.now) });
            self.moved.insert(next, Moved { slot: i, at: Some(ctx.now), seen: false });
            // A count, never who
            ctx.event(None, "moved_up", json!({ "slot": i }));
        }
        true
    }

    /// Answers close: a vote is counted (a final tie waits for the host).
    fn close(&mut self, ctx: &mut KitCtx) {
        self.closed_at = Some(ctx.now);
        if face(ctx.spec) == VOTE {
            let tally = self.count(ctx.spec);
            let tie = matches!(tally.outcome, Outcome::Tie { .. });
            self.phase = if tie { TIE } else { CLOSED }.into();
            self.done = !tie;
            ctx.event(None, if tie { "tie" } else { "closed" }, json!({ "ballots_counted": self.ballots.len() }));
            self.tally = Some(tally);
        } else {
            self.phase = CLOSED.into();
            ctx.event(None, "closed", json!({ "answers": self.answered_count(ctx) }));
            self.done = Self::over(ctx.spec, ctx.now);
        }
    }

    /// The event (else the closing time) has passed: an RSVP or sign-up is final then.
    fn over(spec: &Value, now: DateTime<Utc>) -> bool {
        spec.get("event_at").and_then(spec_date).or_else(|| spec.get("closes_at").and_then(spec_date)).is_none_or(|t| now >= t)
    }

    fn answered_count(&self, ctx: &KitCtx) -> usize {
        if let Some(k) = &self.deleted {
            return k.answered;
        }
        match face(ctx.spec) {
            RSVP => self.answers.len(),
            SIGNUP => self.slots.iter().map(|s| s.taken.len() + s.waiting.len()).sum(),
            _ => self.voted.len(),
        }
    }

    /// Has this player answered (RSVP), taken or queued for a slot (sign-up), or voted?
    fn has_answered(&self, spec: &Value, id: &str) -> bool {
        match face(spec) {
            RSVP => self.answers.contains_key(id),
            SIGNUP => self.slot_of(id).is_some() || self.waiting_of(id).is_some(),
            _ => self.voted.contains_key(id),
        }
    }

    /// Listed members who haven't answered (or voted) yet, as the host typed them.
    fn not_answered(&self, ctx: &KitCtx) -> Vec<String> {
        if self.deleted.is_some() {
            return vec![];
        }
        members(ctx.spec)
            .into_iter()
            .filter(|m| {
                let k = key(m);
                !ctx.players.iter().any(|p| key(&p.name) == k && self.has_answered(ctx.spec, &p.id))
            })
            .collect()
    }

    /// The Chaser: one draft per person who said Maybe, said Yes and isn't ticked paid, just
    /// moved up, or (with a member list) hasn't answered. Rate-limited per person.
    fn drafts(&self, ctx: &KitCtx) -> Vec<Value> {
        let spec = ctx.spec;
        let f = face(spec);
        let open = self.phase == OPEN;
        let mut out: Vec<Nudge> = Vec::new();
        let mut seen = BTreeSet::new();
        for p in ctx.players {
            let kind = match f {
                RSVP => match self.answers.get(&p.id).map(|a| a.answer.as_str()) {
                    Some("maybe") if open => Some("maybe"),
                    Some("yes") if tracks_paid(spec) && !self.done && !self.paid.contains_key(&p.id) => Some("not_paid"),
                    _ => None,
                },
                SIGNUP => self.moved.get(&p.id).filter(|m| open && m.at.is_some_and(|at| self.copied.get(&key(&p.name)).is_none_or(|c| *c < at))).map(|_| "moved_up"),
                _ => None,
            };
            if let Some(kind) = kind {
                let k = key(&p.name);
                seen.insert(k.clone());
                out.push(Nudge { to: p.name.clone(), key: k, player: Some(p), kind, slot: self.moved.get(&p.id).map(|m| m.slot) });
            }
        }
        if open {
            for m in self.not_answered(ctx) {
                let k = key(&m);
                if seen.insert(k.clone()) {
                    let p = ctx.players.iter().find(|p| key(&p.name) == k);
                    out.push(Nudge { to: m, key: k, player: p, kind: "not_answered", slot: None });
                }
            }
        }
        out.into_iter()
            .map(|Nudge { to: name, key: k, player: p, kind, slot }| {
                let copied_at = self.copied.get(&k).copied();
                let next = copied_at.map(|c| c + Duration::hours(NUDGE_HOURS)).filter(|n| *n > ctx.now);
                json!({
                    "to": name,
                    "player_id": p.map(|p| p.id.clone()),
                    "kind": kind,
                    "text": fill(spec, kind, &name, slot),
                    "copied_at": copied_at,
                    "next_copy_at": next,
                    "can_copy": next.is_none(),
                })
            })
            .collect()
    }
}

/// One person the Chaser drafts a note for, before the text is filled in.
struct Nudge<'a> {
    to: String,
    key: String,
    player: Option<&'a Player>,
    kind: &'static str,
    slot: Option<usize>,
}

/// A draft from the spec's template (else a plain default): `{name}`, `{title}`, `{when}`,
/// `{place}`, `{host}`, `{slot}` are filled here; `{link}` stays for the card, which knows the
/// join link it shows.
fn fill(spec: &Value, kind: &str, name: &str, slot: Option<usize>) -> String {
    let t = template(spec, kind).unwrap_or_else(|| default_template(face(spec), kind));
    let slot_name = slot.and_then(|i| slot_specs(spec).get(i).map(|s| s.0.clone())).unwrap_or_default();
    t.replace("{name}", name)
        .replace("{title}", spec_str(spec, "title").unwrap_or(""))
        .replace("{when}", spec_str(spec, "when").unwrap_or(""))
        .replace("{place}", spec_str(spec, "place").unwrap_or(""))
        .replace("{host}", spec_str(spec, "host_name").unwrap_or("the host"))
        .replace("{slot}", &slot_name)
}

fn tally_json(spec: &Value, s: &St) -> Value {
    let Some(t) = &s.tally else { return Value::Null };
    let names = options(spec);
    let name = |o: usize| names.get(o).cloned().unwrap_or_default();
    let (kind, winner, tied) = match &t.outcome {
        Outcome::Winner { option } => ("winner", Some(*option), vec![]),
        Outcome::Tie { options } => (if s.picked { "picked" } else { "tie" }, None, options.clone()),
        Outcome::NoVotes => ("no_votes", None, vec![]),
    };
    let rounds: Vec<Value> = t
        .rounds
        .iter()
        .enumerate()
        .map(|(i, r)| {
            json!({
                "round": i + 1,
                "counts": r.counts.iter().map(|(o, c)| json!({ "option": o, "name": name(*o), "votes": c })).collect::<Vec<_>>(),
                "exhausted": r.exhausted,
                "dropped": r.dropped.iter().map(|o| json!({ "option": o, "name": name(*o) })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let last = t.rounds.last();
    let votes_of = |o: usize| last.and_then(|r| r.counts.iter().find(|(x, _)| *x == o)).map_or(0, |(_, c)| *c);
    json!({
        "outcome": if s.picked { "picked" } else { kind },
        "winner": winner.map(|o| json!({ "option": o, "name": name(o), "votes": votes_of(o) })),
        "tied": tied.iter().map(|o| json!({ "option": o, "name": name(*o), "votes": votes_of(*o) })).collect::<Vec<_>>(),
        "ballots_counted": s.ballots_counted(),
        "final_round_active": last.map(|r| r.counts.iter().map(|(_, c)| c).sum::<u32>()),
        "rounds": rounds,
    })
}

pub struct Gather;

impl Gather {
    fn person(p: &Player) -> Value {
        json!({ "id": p.id, "name": p.name, "avatar": p.avatar })
    }

    /// Facts anyone with the link may see (lobby, info, the top of every view): never a guest's
    /// name, answer, ballot or tick.
    fn facts(ctx: &KitCtx, s: &St) -> Value {
        let spec = ctx.spec;
        let f = face(spec);
        let mut fields: Vec<&str> = vec!["name"];
        fields.push(match f {
            RSVP => "answer",
            SIGNUP => "slot",
            _ => "ballot",
        });
        for extra in ["plus_ones", "food_needs", "slot_note"] {
            if has_field(spec, extra) {
                fields.push(extra);
            }
        }
        let mut v = json!({
            "face": f,
            "phase": s.phase,
            "when": spec_str(spec, "when"),
            "place": spec_str(spec, "place"),
            "host_name": spec_str(spec, "host_name"),
            "price": spec_str(spec, "price"),
            "closes_at": s.closes(spec),
            "event_at": spec.get("event_at").and_then(spec_date),
            "fields": fields,
            "show_names": flag(spec, "show_names"),
            "joined": s.joined(ctx),
            "members": members(spec).len(),
        });
        match f {
            RSVP => {
                v["food_choices"] = json!(if has_field(spec, "food_needs") { FOOD } else { &[] });
                v["max_plus_ones"] = json!(if has_field(spec, "plus_ones") { MAX_PLUS_ONES } else { 0 });
                v["note_max"] = json!(NOTE_MAX);
            }
            SIGNUP => {
                v["waiting_list"] = json!(flag(spec, "waiting_list"));
                v["note_max"] = json!(NOTE_MAX);
                v["slots"] = json!(slot_specs(spec)
                    .iter()
                    .enumerate()
                    .map(|(i, (name, cap))| {
                        let (filled, waiting) = s.slot_counts(i);
                        json!({ "slot": i, "name": name, "cap": cap, "filled": filled, "full": filled >= *cap, "waiting": waiting })
                    })
                    .collect::<Vec<_>>());
            }
            _ => {
                v["show_live_count"] = json!(flag(spec, "show_live_count"));
                v["ballot"] = json!({
                    "kind": ballot_kind(spec),
                    "options": options(spec).iter().enumerate().map(|(i, n)| json!({ "option": i, "name": n })).collect::<Vec<_>>(),
                    "in_play": s.in_play(spec),
                });
                v["voted"] = json!(s.ballots_counted());
            }
        }
        v
    }

    fn rsvp_counts(s: &St) -> Value {
        if let Some(k) = &s.deleted {
            return k.rsvp.clone();
        }
        let n = |a: &str| s.answers.values().filter(|x| x.answer == a).count();
        let plus: u64 = s.answers.values().filter(|x| x.answer == "yes").map(|x| x.plus_ones).sum();
        json!({ "yes": n("yes"), "maybe": n("maybe"), "no": n("no"), "plus_ones": plus, "coming": n("yes") as u64 + plus })
    }

    fn guest_view(ctx: &KitCtx, s: &St, me: &str) -> Value {
        let spec = ctx.spec;
        let mut v = Self::facts(ctx, s);
        let names = flag(spec, "show_names");
        match face(spec) {
            RSVP => {
                let c = Self::rsvp_counts(s);
                // Counts only: plus-ones per person are the host's
                v["counts"] = json!({ "yes": c["yes"], "maybe": c["maybe"], "no": c["no"], "coming": c["coming"] });
                v["people"] = if names {
                    json!(ctx
                        .players
                        .iter()
                        .filter_map(|p| s.answers.get(&p.id).map(|a| {
                            let mut row = Self::person(p);
                            row["answer"] = json!(a.answer);
                            row
                        }))
                        .collect::<Vec<_>>())
                } else {
                    Value::Null
                };
                let mine = s.answers.get(me);
                v["me"] = json!({
                    "answer": mine.map(|a| a.answer.clone()),
                    "plus_ones": mine.map(|a| a.plus_ones),
                    "food": mine.map(|a| a.food.clone()),
                    "food_other": mine.and_then(|a| a.food_other.clone()),
                    "answered_at": mine.and_then(|a| a.at),
                    // Only their own, and only once the host ticked it
                    "paid": s.paid.get(me).map(|at| json!({ "at": at })),
                });
            }
            SIGNUP => {
                let caps = slot_specs(spec);
                v["slots"] = json!(caps
                    .iter()
                    .enumerate()
                    .map(|(i, (name, cap))| {
                        let slot = s.slots.get(i).cloned().unwrap_or_default();
                        let people: Vec<Value> = slot
                            .taken
                            .iter()
                            .filter_map(|t| {
                                let p = ctx.player(&t.id)?;
                                let mut row = Self::person(p);
                                row["note"] = json!(t.note);
                                Some(row)
                            })
                            .collect();
                        let waiting: Vec<Value> = slot.waiting.iter().filter_map(|w| ctx.player(w).map(Self::person)).collect();
                        let (filled, queued) = s.slot_counts(i);
                        json!({
                            "slot": i, "name": name, "cap": cap, "filled": filled, "full": filled >= *cap,
                            "people": if names { json!(people) } else { Value::Null },
                            "waiting": queued,
                            "waiting_people": if names { json!(waiting) } else { Value::Null },
                        })
                    })
                    .collect::<Vec<_>>());
                let slot = s.slot_of(me);
                v["me"] = json!({
                    "slot": slot,
                    "note": slot.and_then(|i| s.slots[i].taken.iter().find(|t| t.id == me).and_then(|t| t.note.clone())),
                    "waiting": s.waiting_of(me).map(|(i, p)| json!({ "slot": i, "position": p + 1 })),
                    "moved_up": s.moved.get(me).map(|m| json!({ "slot": m.slot, "at": m.at, "new": !m.seen })),
                });
            }
            _ => {
                let mine = s.voted.contains_key(me).then(|| s.ballots.get(&s.ballot_key(me)).cloned()).flatten();
                v["me"] = json!({ "voted": s.voted.contains_key(me), "ballot": mine });
                let live = flag(spec, "show_live_count") && s.voted.contains_key(me) && s.phase == OPEN;
                v["live"] = if live { json!(s.first_choices(spec)) } else { Value::Null };
                v["result"] = tally_json(spec, s);
            }
        }
        v
    }

    /// The host's view: everything a guest sees with names, plus the host-only data (paid
    /// ticks, food needs, plus-ones) and the Chaser's drafts. Never a person's ballot.
    fn host_view(ctx: &KitCtx, s: &St) -> Value {
        let spec = ctx.spec;
        let mut v = Self::facts(ctx, s);
        // The pocket's own title and deletion date (the core's `pocket.title` and
        // `pocket.expires_at`, from the same `title()` and retention rule), so the chat card's
        // `gather` is this view as it is. Host only: the guest page reads them from `pocket`.
        v["title"] = json!(Gather.title(spec));
        v["expires_at"] = json!(Gather.rules(spec).retention.expires_at(spec, None));
        match face(spec) {
            RSVP => {
                v["counts"] = Self::rsvp_counts(s);
                let mut food: BTreeMap<String, u64> = FOOD.iter().map(|f| (f.to_string(), 0)).collect();
                let mut other = Vec::new();
                for a in s.answers.values().filter(|a| a.answer == "yes") {
                    for f in &a.food {
                        *food.entry(f.clone()).or_default() += 1;
                    }
                    other.extend(a.food_other.clone());
                }
                v["food"] = json!({ "counts": food, "other": other });
                v["people"] = json!(ctx
                    .players
                    .iter()
                    .map(|p| {
                        let a = s.answers.get(&p.id);
                        let mut row = Self::person(p);
                        row["answer"] = json!(a.map(|a| a.answer.clone()));
                        row["plus_ones"] = json!(a.map(|a| a.plus_ones));
                        row["food"] = json!(a.map(|a| a.food.clone()));
                        row["food_other"] = json!(a.and_then(|a| a.food_other.clone()));
                        row["answered_at"] = json!(a.and_then(|a| a.at));
                        row["paid_at"] = json!(s.paid.get(&p.id));
                        row
                    })
                    .collect::<Vec<_>>());
                let yes: Vec<&String> = s.answers.iter().filter(|(_, a)| a.answer == "yes").map(|(id, _)| id).collect();
                v["paid"] = if tracks_paid(spec) {
                    json!({
                        "ticked": yes.iter().filter(|id| s.paid.contains_key(id.as_str())).count(),
                        "of": yes.len(),
                        "not_yet": ctx.players.iter().filter(|p| yes.contains(&&p.id) && !s.paid.contains_key(&p.id)).map(|p| p.name.clone()).collect::<Vec<_>>(),
                    })
                } else {
                    Value::Null
                };
            }
            SIGNUP => {
                let caps = slot_specs(spec);
                v["slots"] = json!(caps
                    .iter()
                    .enumerate()
                    .map(|(i, (name, cap))| {
                        let slot = s.slots.get(i).cloned().unwrap_or_default();
                        let people: Vec<Value> = slot
                            .taken
                            .iter()
                            .filter_map(|t| {
                                let p = ctx.player(&t.id)?;
                                let mut row = Self::person(p);
                                row["note"] = json!(t.note);
                                row["at"] = json!(t.at);
                                Some(row)
                            })
                            .collect();
                        let waiting: Vec<Value> = slot.waiting.iter().filter_map(|w| ctx.player(w).map(Self::person)).collect();
                        let (filled, queued) = s.slot_counts(i);
                        json!({ "slot": i, "name": name, "cap": cap, "filled": filled, "full": filled >= *cap, "people": people, "waiting": queued, "waiting_people": waiting })
                    })
                    .collect::<Vec<_>>());
                v["moved_up"] = json!(ctx
                    .players
                    .iter()
                    .filter_map(|p| s.moved.get(&p.id).map(|m| json!({ "id": p.id, "name": p.name, "slot": m.slot, "at": m.at, "seen": m.seen })))
                    .collect::<Vec<_>>());
            }
            _ => {
                // Who voted and when; no ballot, no order
                v["voters"] = json!(ctx
                    .players
                    .iter()
                    .filter_map(|p| s.voted.get(&p.id).map(|at| {
                        let mut row = Self::person(p);
                        row["at"] = json!(at);
                        row
                    }))
                    .collect::<Vec<_>>());
                v["live"] = if s.phase == OPEN { json!(s.first_choices(spec)) } else { Value::Null };
                v["result"] = tally_json(spec, s);
            }
        }
        v["not_answered"] = json!(s.not_answered(ctx));
        // pa10c: the count stays after "Delete answers now", when the list above is empty
        v["not_answered_count"] = json!(s.not_answered_count(ctx));
        v["drafts"] = json!(s.drafts(ctx));
        v["needs_you"] = json!(s.phase == TIE);
        v["summary"] = json!(Self::summary_line(ctx, s));
        v["answers_deleted_at"] = json!(s.deleted.as_ref().and_then(|k| k.at));
        v
    }

    /// One line for the card foot and Done: counts and the winner, never a name of a guest.
    fn summary_line(ctx: &KitCtx, s: &St) -> String {
        if let Some(k) = &s.deleted {
            return k.summary.clone();
        }
        let spec = ctx.spec;
        match face(spec) {
            RSVP => {
                let c = Self::rsvp_counts(s);
                format!("{} coming · {} maybe · {} no", c["coming"], c["maybe"], c["no"])
            }
            SIGNUP => {
                let caps = slot_specs(spec);
                let filled: usize = s.slots.iter().map(|x| x.taken.len()).sum();
                let total: usize = caps.iter().map(|c| c.1).sum();
                let waiting: usize = s.slots.iter().map(|x| x.waiting.len()).sum();
                if waiting > 0 {
                    format!("{filled} of {total} spots taken · {waiting} waiting")
                } else {
                    format!("{filled} of {total} spots taken")
                }
            }
            _ => {
                let t = tally_json(spec, s);
                match (s.phase.as_str(), t["outcome"].as_str()) {
                    (OPEN, _) => format!("{} voted", s.voted.len()),
                    (TIE, _) => {
                        let tied: Vec<String> = t["tied"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
                        format!("Tie: {}", tied.join(" and "))
                    }
                    (_, Some("no_votes")) => "Nobody voted".to_string(),
                    (_, Some("picked")) => format!("{} (picked after a tie)", t["winner"]["name"].as_str().unwrap_or("")),
                    _ => {
                        let w = t["winner"]["name"].as_str().unwrap_or("");
                        let rounds = s.tally.as_ref().map_or(0, |x| x.rounds.len());
                        let last = s.tally.as_ref().and_then(|x| x.rounds.last());
                        match last.map(|r| r.counts.as_slice()) {
                            Some([(_, a), (_, b), ..]) if rounds > 1 => format!("{w} wins, {a} to {b} in the last round"),
                            Some([(_, a), ..]) => format!("{w} wins with {a} of {}", s.ballots.len()),
                            _ => format!("{w} wins"),
                        }
                    }
                }
            }
        }
    }
}

fn check_text(spec: &Value, f: &str, max: usize) -> Result<(), String> {
    match spec.get(f) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(t)) if t.chars().count() <= max => Ok(()),
        Some(Value::String(_)) => Err(format!("{f} is at most {max} characters.")),
        Some(_) => Err(format!("{f} must be text.")),
    }
}

fn check_bool(spec: &Value, f: &str) -> Result<(), String> {
    match spec.get(f) {
        None | Some(Value::Null) | Some(Value::Bool(_)) => Ok(()),
        Some(_) => Err(format!("{f} must be true or false.")),
    }
}

fn validate(spec: &Value, now: DateTime<Utc>) -> Result<(), String> {
    let obj: &Map<String, Value> = spec.as_object().ok_or("The spec must be a JSON object.")?;
    if let Some(k) = obj.keys().find(|k| !SPEC_KEYS.contains(&k.as_str())) {
        return Err(format!("{k} isn't something this pocket app knows. It takes: {}.", SPEC_KEYS.join(", ")));
    }
    let f = face(spec);
    if ![RSVP, SIGNUP, VOTE].contains(&f) {
        return Err("face must be rsvp, signup or vote.".into());
    }
    match spec_str(spec, "title") {
        None => return Err("The spec needs a title.".into()),
        Some(t) if t.chars().count() > 80 => return Err("title is at most 80 characters.".into()),
        _ => {}
    }
    check_text(spec, "when", 60)?;
    if spec_str(spec, "when").is_some_and(|w| chrono::DateTime::parse_from_rfc3339(w).is_ok()) {
        return Err("when is the line people read, like \"Sat, Dec 12, 7:30 PM\". Put the event's time in event_at.".into());
    }
    check_text(spec, "place", 60)?;
    check_text(spec, "host_name", 40)?;
    check_text(spec, "price", 24)?;
    if spec_str(spec, "price").is_some() && f != RSVP {
        return Err("price is for an RSVP (the host ticks who paid).".into());
    }
    let date = |name: &str| -> Result<Option<DateTime<Utc>>, String> {
        match spec.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => spec_date(v).map(Some).ok_or_else(|| format!("{name} must be a date (YYYY-MM-DD) or an RFC 3339 time.")),
        }
    };
    let closes = date("closes_at")?.ok_or("The spec needs closes_at, when answers close.")?;
    if closes <= now {
        return Err("closes_at is already past. Pick a time when answers should close.".into());
    }
    if let Some(e) = date("event_at")? {
        if e < closes {
            return Err("event_at is before closes_at. Answers close on or before the event.".into());
        }
    }
    for b in ["show_names", "show_live_count", "waiting_list"] {
        check_bool(spec, b)?;
    }
    if let Some(fields) = spec.get("fields").filter(|v| !v.is_null()) {
        let list = fields.as_array().ok_or("fields must be a list.")?;
        let core = match f {
            RSVP => "answer",
            SIGNUP => "slot",
            _ => "ballot",
        };
        let extras: &[&str] = match f {
            RSVP => &["plus_ones", "food_needs"],
            SIGNUP => &["slot_note"],
            _ => &[],
        };
        for x in list {
            let x = x.as_str().ok_or("fields must be a list of field names.")?;
            if x != "name" && x != core && !extras.contains(&x) {
                return Err(match extras.is_empty() {
                    true => format!("A {f} has no field {x:?}: its guests give a name and a ballot."),
                    false => format!("A {f} has no field {x:?}. It can add: {}.", extras.join(", ")),
                });
            }
        }
    }
    match (f, spec.get("slots").filter(|v| !v.is_null())) {
        (SIGNUP, None) => return Err("A sign-up needs slots, like [{\"name\": \"Desserts\", \"cap\": 2}].".into()),
        (SIGNUP, Some(slots)) => {
            let slots = slots.as_array().ok_or("slots must be a list.")?;
            if slots.is_empty() || slots.len() > MAX_SLOTS {
                return Err(format!("A sign-up has 1 to {MAX_SLOTS} slots."));
            }
            let mut names = BTreeSet::new();
            for s in slots {
                let name = spec_str(s, "name").ok_or("Each slot needs a name.")?;
                if name.chars().count() > 40 {
                    return Err("A slot's name is at most 40 characters.".into());
                }
                if !names.insert(name.to_lowercase()) {
                    return Err(format!("Two slots are called {name:?}."));
                }
                match s.get("cap").and_then(Value::as_u64) {
                    Some(c) if (1..=DEFAULT_MAX_PLAYERS as u64).contains(&c) => {}
                    _ => return Err(format!("{name}: cap must be a whole number from 1 to {DEFAULT_MAX_PLAYERS}.")),
                }
            }
        }
        (_, Some(_)) => return Err("slots are for a sign-up.".into()),
        _ => {}
    }
    match (f, spec.get("ballot").filter(|v| !v.is_null())) {
        (VOTE, None) => return Err("A vote needs a ballot, like {\"kind\": \"ranked\", \"options\": [\"The Old Mill\", \"Casa Lupa\"]}.".into()),
        (VOTE, Some(b)) => {
            match b.get("kind").and_then(Value::as_str) {
                Some(ONE_PICK) | Some(RANKED) => {}
                _ => return Err("ballot.kind must be one_pick or ranked.".into()),
            }
            let opts = b.get("options").and_then(Value::as_array).ok_or("ballot.options must be a list of choices.")?;
            if opts.len() < MIN_OPTIONS || opts.len() > MAX_OPTIONS {
                return Err(format!("A vote has {MIN_OPTIONS} to {MAX_OPTIONS} options."));
            }
            let mut seen = BTreeSet::new();
            for o in opts {
                let o = o.as_str().map(str::trim).filter(|o| !o.is_empty()).ok_or("Each option is a short text.")?;
                if o.chars().count() > 60 {
                    return Err("An option is at most 60 characters.".into());
                }
                if !seen.insert(o.to_lowercase()) {
                    return Err(format!("{o:?} is in the options twice."));
                }
            }
            if let Some(k) = b.as_object().and_then(|m| m.keys().find(|k| *k != "kind" && *k != "options")) {
                return Err(format!("ballot.{k} isn't something a vote knows (kind, options)."));
            }
        }
        (_, Some(_)) => return Err("ballot is for a vote.".into()),
        _ => {}
    }
    if let Some(m) = spec.get("members").filter(|v| !v.is_null()) {
        let names = m.as_array().ok_or("members must be a list of names.")?;
        if names.len() > DEFAULT_MAX_PLAYERS {
            return Err(format!("members holds at most {DEFAULT_MAX_PLAYERS} names."));
        }
        for n in names {
            let n = n.as_str().ok_or("members must be a list of names.")?;
            words::clean_name(n).map_err(|e| format!("{n:?} in members: {}", e.message()))?;
        }
    }
    if let Some(t) = spec.get("templates").filter(|v| !v.is_null()) {
        let t = t.as_object().ok_or("templates must be an object of notes (maybe, not_paid, moved_up, not_answered).")?;
        for (k, v) in t {
            if !NUDGES.contains(&k.as_str()) {
                return Err(format!("templates.{k} isn't a nudge this pocket drafts ({}).", NUDGES.join(", ")));
            }
            let text = v.as_str().ok_or(format!("templates.{k} must be text."))?;
            if text.chars().count() > TEMPLATE_MAX {
                return Err(format!("templates.{k} is at most {TEMPLATE_MAX} characters."));
            }
        }
    }
    Ok(())
}

impl Kit for Gather {
    fn id(&self) -> &'static str {
        ID
    }

    fn rules(&self, _spec: &Value) -> KitRules {
        KitRules {
            retention: Retention::AfterDate { fields: DATE_FIELDS, days: RETENTION_DAYS },
            needs_pin: true,
            max_players: DEFAULT_MAX_PLAYERS,
            guest_actions: &["answer", "take", "leave", "let_go", "seen", "vote"],
            host_actions: &["set_paid", "copied", "pick", "revote_24h", "close_now", "delete_answers"],
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        validate(spec, Utc::now())
    }

    fn init(&self, spec: &Value) -> Value {
        let s = St {
            v: 1,
            phase: OPEN.into(),
            slots: vec![Slot::default(); slot_specs(spec).len()],
            ballot_salt: random_hex(16),
            ..St::default()
        };
        serde_json::to_value(&s).unwrap_or(Value::Null)
    }

    fn admit(&self, ctx: &KitCtx, state: &Value, _name_key: &str) -> Result<(), KitError> {
        if load(state).taking_answers(ctx.spec, ctx.now) {
            Ok(())
        } else {
            Err(not_now("Answers are closed. If you answered before, open your page with your name and PIN."))
        }
    }

    fn on_remove(&self, ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        let mut s = load(state);
        s.answers.remove(player_id);
        s.paid.remove(player_id);
        let caps = slot_specs(ctx.spec);
        if s.phase == OPEN {
            s.leave(ctx, player_id, &caps);
        } else {
            for slot in s.slots.iter_mut() {
                slot.taken.retain(|t| t.id != player_id);
                slot.waiting.retain(|w| w != player_id);
            }
            s.moved.remove(player_id);
        }
        if s.voted.remove(player_id).is_some() && s.phase == OPEN {
            let k = s.ballot_key(player_id);
            s.ballots.remove(&k);
        }
        store(state, &s);
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let mut s = load(state);
        let now = ctx.now;
        let spec = ctx.spec;
        let f = face(spec);
        let wrong_face = |want: &str| bad(format!("This pocket is a {f}; {name} is for a {want}."));
        if let Viewer::Guest(_) = actor {
            if name != "seen" && !s.taking_answers(spec, now) {
                return Err(not_now(match s.phase.as_str() {
                    TIE => "Voting has closed: it's a tie, and the host picks.",
                    _ => "Answers are closed.",
                }));
            }
        }
        // pa10c: after "Delete answers now" nothing may bring a name back (a Chaser copy, a
        // paid tick); only the delete itself repeats.
        if s.deleted.is_some() && name != "delete_answers" {
            return Err(not_now("Answers were deleted."));
        }
        match (actor, name) {
            (Viewer::Guest(me), "answer") => {
                if f != RSVP {
                    return Err(wrong_face(RSVP));
                }
                let answer = body.get("answer").and_then(Value::as_str).filter(|a| ["yes", "maybe", "no"].contains(a)).ok_or_else(|| bad("answer is yes, maybe or no"))?;
                let plus_ones = match body.get("plus_ones").filter(|v| !v.is_null()) {
                    None => 0,
                    Some(_) if !has_field(spec, "plus_ones") => return Err(bad("This RSVP doesn't ask for plus-ones.")),
                    Some(v) => v.as_u64().filter(|n| *n <= MAX_PLUS_ONES).ok_or_else(|| bad(format!("plus_ones is 0 to {MAX_PLUS_ONES}")))?,
                };
                let mut food = Vec::new();
                if let Some(list) = body.get("food").filter(|v| !v.is_null()) {
                    if !has_field(spec, "food_needs") {
                        return Err(bad("This RSVP doesn't ask for food needs."));
                    }
                    for x in list.as_array().ok_or_else(|| bad("food is a list"))? {
                        let x = x.as_str().filter(|x| FOOD.contains(x)).ok_or_else(|| bad(format!("food choices are: {}", FOOD.join(", "))))?;
                        if !food.iter().any(|y| y == x) {
                            food.push(x.to_string());
                        }
                    }
                }
                let food_other = match body.get("food_other").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                    None => None,
                    Some(_) if !has_field(spec, "food_needs") => return Err(bad("This RSVP doesn't ask for food needs.")),
                    Some(t) => Some(words::clean_text(t, NOTE_MAX).map_err(|e| bad(e.message()))?),
                };
                // Plus-ones and food matter only for people coming
                let coming = answer == "yes";
                s.answers.insert(
                    me.clone(),
                    Answer {
                        answer: answer.into(),
                        plus_ones: if coming { plus_ones } else { 0 },
                        food: if coming { food } else { vec![] },
                        food_other: if coming { food_other } else { None },
                        at: Some(now),
                    },
                );
                if !coming {
                    s.paid.remove(me);
                }
            }
            (Viewer::Guest(me), "take") => {
                if f != SIGNUP {
                    return Err(wrong_face(SIGNUP));
                }
                let caps = slot_specs(spec);
                let i = body.get("slot").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < caps.len()).ok_or_else(|| bad("slot is the number of one of the slots"))?;
                let note = match body.get("note").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                    None => None,
                    Some(_) if !has_field(spec, "slot_note") => return Err(bad("This sign-up doesn't take notes.")),
                    Some(t) => Some(words::clean_text(t, NOTE_MAX).map_err(|e| bad(e.message()))?),
                };
                if s.slot_of(me) == Some(i) {
                    if let Some(t) = s.slots[i].taken.iter_mut().find(|t| t.id == *me) {
                        t.note = note;
                    }
                } else if s.waiting_of(me).map(|w| w.0) == Some(i) {
                    // Already in line for it
                } else {
                    let full = s.slots[i].taken.len() >= caps[i].1;
                    if full && !flag(spec, "waiting_list") {
                        return Err(not_now(format!("{} is full.", caps[i].0)));
                    }
                    s.leave(ctx, me, &caps);
                    if full {
                        s.slots[i].waiting.push(me.clone());
                    } else {
                        s.slots[i].taken.push(Taken { id: me.clone(), note, at: Some(now) });
                    }
                }
            }
            (Viewer::Guest(me), "leave" | "let_go") => {
                if f != SIGNUP {
                    return Err(wrong_face(SIGNUP));
                }
                let caps = slot_specs(spec);
                if !s.leave(ctx, me, &caps) {
                    return Err(not_now("You're not in a slot or on a waiting list."));
                }
            }
            (Viewer::Guest(me), "seen") => {
                if let Some(m) = s.moved.get_mut(me) {
                    m.seen = true;
                }
            }
            (Viewer::Guest(me), "vote") => {
                if f != VOTE {
                    return Err(wrong_face(VOTE));
                }
                let in_play = s.in_play(spec);
                let ranking: Vec<usize> = body
                    .get("ranking")
                    .and_then(Value::as_array)
                    .ok_or_else(|| bad("ranking is a list of option numbers, 1st choice first"))?
                    .iter()
                    .map(|x| x.as_u64().map(|x| x as usize).filter(|x| in_play.contains(x)).ok_or_else(|| bad("ranking names an option that isn't on the ballot")))
                    .collect::<Result<_, _>>()?;
                if ranking.is_empty() {
                    return Err(bad("Pick at least one option."));
                }
                if ranking.iter().collect::<BTreeSet<_>>().len() != ranking.len() {
                    return Err(bad("Each option can be ranked once."));
                }
                if ballot_kind(spec) == ONE_PICK && ranking.len() != 1 {
                    return Err(bad("This vote takes one pick."));
                }
                let k = s.ballot_key(me);
                s.ballots.insert(k, ranking);
                s.voted.insert(me.clone(), now);
                // No event: an event row next to a ballot change would pair the two
            }
            (Viewer::Host, "set_paid") => {
                if !tracks_paid(spec) {
                    return Err(bad("This pocket doesn't track who paid (an RSVP with a price does)."));
                }
                let id = body.get("player_id").and_then(Value::as_str).ok_or_else(|| bad("player_id is required"))?;
                if ctx.player(id).is_none() {
                    return Err(bad("That person isn't in this pocket."));
                }
                let paid = body.get("paid").and_then(Value::as_bool).unwrap_or(true);
                if paid {
                    if s.answers.get(id).map(|a| a.answer.as_str()) != Some("yes") {
                        return Err(not_now("Only people who said Yes can be ticked as paid."));
                    }
                    s.paid.entry(id.to_string()).or_insert(now);
                } else {
                    s.paid.remove(id);
                }
            }
            (Viewer::Host, "copied") => {
                let to = body.get("to").and_then(Value::as_str).ok_or_else(|| bad("to is the name on the draft"))?;
                let k = key(to);
                let drafts = s.drafts(ctx);
                let d = drafts.iter().find(|d| d["to"].as_str().map(key).as_deref() == Some(k.as_str())).ok_or_else(|| not_now(format!("There's no note for {to} right now.")))?;
                if let Some(until) = d["next_copy_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) {
                    // The view carries the same time as next_copy_at
                    return Err(not_now(format!("One nudge per person every {NUDGE_HOURS} hours. The next one for {to} is from {}.", until.with_timezone(&Utc).to_rfc3339())));
                }
                s.copied.insert(k, now);
            }
            (Viewer::Host, "pick") => {
                if s.phase != TIE {
                    return Err(not_now("There's no tie to pick from."));
                }
                let o = body.get("option").and_then(Value::as_u64).map(|o| o as usize).ok_or_else(|| bad("option is required"))?;
                let tied = match s.tally.as_ref().map(|t| &t.outcome) {
                    Some(Outcome::Tie { options }) => options.clone(),
                    _ => vec![],
                };
                if !tied.contains(&o) {
                    return Err(bad("Pick one of the tied options."));
                }
                if let Some(t) = s.tally.as_mut() {
                    t.outcome = Outcome::Winner { option: o };
                }
                s.picked = true;
                s.phase = CLOSED.into();
                s.done = true;
                ctx.event(None, "picked", json!({}));
            }
            (Viewer::Host, "revote_24h") => {
                if s.phase != TIE {
                    return Err(not_now("Voting again is for a tie."));
                }
                let tied = match s.tally.as_ref().map(|t| &t.outcome) {
                    Some(Outcome::Tie { options }) => options.clone(),
                    _ => vec![],
                };
                s.only = tied;
                s.ballots.clear();
                s.voted.clear();
                s.ballot_salt = random_hex(16);
                s.tally = None;
                s.phase = OPEN.into();
                s.closed_at = None;
                s.closes_at = Some(now + Duration::hours(REVOTE_HOURS));
                ctx.event(None, "revote", json!({ "options": s.only.len() }));
            }
            (Viewer::Host, "close_now") => {
                if s.phase != OPEN {
                    return Err(not_now("Answers are already closed."));
                }
                s.close(ctx);
            }
            (Viewer::Host, "delete_answers") => {
                match s.phase.as_str() {
                    CLOSED => {}
                    TIE => return Err(not_now("Pick the winner or vote again first.")),
                    _ => return Err(not_now("Close answers first.")),
                }
                if s.deleted.is_none() {
                    // A count, never who
                    ctx.event(None, "answers_deleted", json!({ "answers": s.answered_count(ctx) }));
                }
                s.delete_answers(ctx);
            }
            _ => return Err(bad(format!("unknown action {name}"))),
        }
        store(state, &s);
        Ok(())
    }

    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let mut s = load(state);
        let now = ctx.now;
        let changed = if s.phase == OPEN && s.closes(ctx.spec).is_some_and(|c| now >= c) {
            s.close(ctx);
            true
        } else if s.phase == CLOSED && !s.done && St::over(ctx.spec, now) {
            s.done = true;
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

    /// pa10c: a decided vote or a past event ends the pocket, and its data stays until the
    /// deletion date; "Delete answers now" still works then.
    fn host_actions_after_end(&self) -> &'static [&'static str] {
        &["delete_answers"]
    }

    fn forgets_everyone(&self, state: &Value) -> bool {
        load(state).deleted.is_some()
    }

    fn kept_players(&self, state: &Value) -> Option<usize> {
        load(state).deleted.map(|k| k.joined)
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let s = load(state);
        match viewer {
            Viewer::Guest(me) => Self::guest_view(ctx, &s, me),
            Viewer::Host => Self::host_view(ctx, &s),
            // No big screen: the public facts only
            Viewer::BigScreen => Self::facts(ctx, &s),
        }
    }

    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        Self::facts(ctx, &load(state))
    }

    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        let mut v = json!({
            "face": face(ctx.spec),
            "phase": s.phase,
            "joined": s.joined(ctx),
            "answered": s.answered_count(ctx),
            "not_answered": s.not_answered_count(ctx),
            "drafts": s.drafts(ctx).len(),
            "needs_you": s.phase == TIE,
            "summary": Self::summary_line(ctx, &s),
            "answers_deleted_at": s.deleted.as_ref().and_then(|k| k.at),
        });
        if face(ctx.spec) == RSVP {
            v["counts"] = Self::rsvp_counts(&s);
        }
        v
    }

    fn finished(&self, state: &Value) -> bool {
        load(state).done
    }

    /// A decided vote as pa7's results card: the options as "teams", winner first, with their
    /// final-round votes; never a name. An RSVP or sign-up has no teams: Null (the card and
    /// Done show the summary line from `host_summary`).
    fn results(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        if face(ctx.spec) != VOTE {
            return Value::Null;
        }
        let Some(t) = s.tally.as_ref() else { return Value::Null };
        let Outcome::Winner { option: w } = t.outcome else { return Value::Null };
        let names = options(ctx.spec);
        let last = t.rounds.last().map(|r| r.counts.clone()).unwrap_or_default();
        let mut order: Vec<(usize, u32)> = last.clone();
        if let Some(k) = order.iter().position(|(o, _)| *o == w) {
            let win = order.remove(k);
            order.insert(0, win);
        } else {
            order.insert(0, (w, 0));
        }
        let teams: Vec<Value> = order.iter().take(12).map(|(o, c)| json!({ "name": names.get(*o).cloned().unwrap_or_default(), "players": null, "points": c, "palette": null, "members": [] })).collect();
        json!({ "teams": teams, "margin": null, "missed": [], "near_misses": null, "near_misses_accepted": null, "summary": Self::summary_line(ctx, &s) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: &[usize], n: usize) -> Vec<Vec<usize>> {
        vec![v.to_vec(); n]
    }

    /// The design's sample: Old Mill 0, Casa Lupa 1, Golden Lotus 2, Pizza 3.
    /// Round 1 6/5/3/1, round 2 6/6/3, round 3 Old Mill 8 to 7.
    #[test]
    fn the_designs_dinner_vote_counts_in_three_rounds() {
        let mut ballots = Vec::new();
        ballots.extend(b(&[0, 1], 6));
        ballots.extend(b(&[1, 0], 5));
        ballots.extend(b(&[2, 0], 2));
        ballots.extend(b(&[2, 1], 1));
        ballots.extend(b(&[3, 1], 1));
        let t = count_ranked(&ballots, &[0, 1, 2, 3]);
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
        let counts: Vec<Vec<(usize, u32)>> = t.rounds.iter().map(|r| r.counts.clone()).collect();
        assert_eq!(counts, vec![vec![(0, 6), (1, 5), (2, 3), (3, 1)], vec![(0, 6), (1, 6), (2, 3)], vec![(0, 8), (1, 7)]]);
        assert_eq!(t.rounds[0].dropped, vec![3]);
        assert_eq!(t.rounds[1].dropped, vec![2]);
        assert!(t.rounds[2].dropped.is_empty());
    }

    #[test]
    fn exhausted_ballots_leave_the_majority_count() {
        // A 4, B 3, C 2 (C voters ranked only C): C drops, its ballots are exhausted, A wins 4 of 7
        let mut ballots = b(&[0], 4);
        ballots.extend(b(&[1], 3));
        ballots.extend(b(&[2], 2));
        let t = count_ranked(&ballots, &[0, 1, 2]);
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
        assert_eq!(t.rounds[1].exhausted, 2);
        assert_eq!(t.rounds[1].counts, vec![(0, 4), (1, 3)]);
    }

    #[test]
    fn a_tie_for_last_drops_the_one_with_fewer_first_choices() {
        // Round 1: A 4, B 3, C 2, D 2 → C and D both drop (same 1st choices), leaving A and B.
        // C's ballots go to B, D's to A: round 2 A 6, B 5.
        let mut ballots = b(&[0], 4);
        ballots.extend(b(&[1], 3));
        ballots.extend(b(&[2, 1], 2));
        ballots.extend(b(&[3, 0], 2));
        let t = count_ranked(&ballots, &[0, 1, 2, 3]);
        assert_eq!(t.rounds[0].dropped, vec![2, 3]);
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
        // Round 2 ties B and C for last, but C had fewer 1st choices: only C drops
        // R1: A 5, B 3, C 2, D 1 (D → C). R2: A 5, B 3, C 3 → C drops (2 < 3 first choices).
        let mut ballots = b(&[0], 5);
        ballots.extend(b(&[1], 3));
        ballots.extend(b(&[2, 1], 2));
        ballots.extend(b(&[3, 2], 1));
        let t = count_ranked(&ballots, &[0, 1, 2, 3]);
        assert_eq!(t.rounds[1].counts, vec![(0, 5), (1, 3), (2, 3)]);
        assert_eq!(t.rounds[1].dropped, vec![2]);
        // R3: C's two ballots go to B; D's ballot ranked only D and C, so it is exhausted.
        // A 5, B 5 is a tie for last again, and B had fewer 1st choices (3 < 5): B drops, A wins.
        assert_eq!(t.rounds[2].counts, vec![(0, 5), (1, 5)]);
        assert_eq!(t.rounds[2].exhausted, 1);
        assert_eq!(t.rounds[2].dropped, vec![1]);
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
        // The mirror: now the lower-numbered option has fewer 1st choices, so option order
        // alone can't pass both cases. R1: A 5, B 2, C 3, D 1 (D → B). R2: A 5, B 3, C 3 →
        // B drops (2 < 3 first choices). R3: B's ballots go to C: A 5, C 6 → C wins.
        let mut ballots = b(&[0], 5);
        ballots.extend(b(&[1, 2], 2));
        ballots.extend(b(&[2], 3));
        ballots.extend(b(&[3, 1, 2], 1));
        let t = count_ranked(&ballots, &[0, 1, 2, 3]);
        assert_eq!(t.rounds[0].dropped, vec![3]);
        assert_eq!(t.rounds[1].counts, vec![(0, 5), (1, 3), (2, 3)]);
        assert_eq!(t.rounds[1].dropped, vec![1]);
        assert_eq!(t.rounds[2].counts, vec![(2, 6), (0, 5)]);
        assert_eq!(t.outcome, Outcome::Winner { option: 2 });
    }

    #[test]
    fn a_two_way_final_with_equal_first_choices_is_a_tie_and_one_leader_over_two_tied_wins() {
        let mut ballots = b(&[0, 1], 3);
        ballots.extend(b(&[1, 0], 3));
        let t = count_ranked(&ballots, &[0, 1]);
        assert_eq!(t.outcome, Outcome::Tie { options: vec![0, 1] });
        assert_eq!(t.rounds.len(), 1);
        // Three-way, all equal: a 3-way final tie
        let mut ballots = b(&[0], 2);
        ballots.extend(b(&[1], 2));
        ballots.extend(b(&[2], 2));
        assert_eq!(count_ranked(&ballots, &[0, 1, 2]).outcome, Outcome::Tie { options: vec![0, 1, 2] });
        // A 5 with B 3 and C 3 behind (equal 1st choices): both trailers drop, A wins
        let mut ballots = b(&[0], 5);
        ballots.extend(b(&[1], 3));
        ballots.extend(b(&[2], 3));
        let t = count_ranked(&ballots, &[0, 1, 2]);
        assert_eq!(t.rounds[0].dropped, vec![1, 2]);
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
        // Equal final round, but A had more 1st choices: B drops and A wins
        // R1: A 4, B 3, C 2 (C → B). R2: A 4, B 5? No: make R2 equal: C → B gives 5. Use C 1.
        let mut ballots = b(&[0], 4);
        ballots.extend(b(&[1], 3));
        ballots.extend(b(&[2, 1], 1));
        let t = count_ranked(&ballots, &[0, 1, 2]);
        assert_eq!(t.rounds[1].counts, vec![(0, 4), (1, 4)]);
        assert_eq!(t.rounds[1].dropped, vec![1], "B had 3 first choices to A's 4");
        assert_eq!(t.outcome, Outcome::Winner { option: 0 });
    }

    #[test]
    fn one_pick_is_plurality_with_ties_and_no_votes() {
        let mut ballots = b(&[0], 3);
        ballots.extend(b(&[1], 2));
        ballots.extend(b(&[2], 2));
        assert_eq!(count_one_pick(&ballots, &[0, 1, 2]).outcome, Outcome::Winner { option: 0 }, "3 of 7 wins without a majority");
        ballots.extend(b(&[1], 1));
        assert_eq!(count_one_pick(&ballots, &[0, 1, 2]).outcome, Outcome::Tie { options: vec![0, 1] });
        assert_eq!(count_one_pick(&[], &[0, 1]).outcome, Outcome::NoVotes);
        assert_eq!(count_ranked(&[], &[0, 1]).outcome, Outcome::NoVotes);
        // A vote again among the tied: ballots naming dropped options are skipped
        assert_eq!(count_one_pick(&b(&[2], 5), &[0, 1]).outcome, Outcome::NoVotes);
    }

    #[test]
    fn a_ranked_count_always_ends_and_never_drops_a_majority() {
        // Small exhaustive-ish sweep: every ballot set of up to 7 ballots over 3 options from a
        // fixed pool ends with a winner who had a majority in the last round, or a tie
        let pool: Vec<Vec<usize>> = vec![vec![0], vec![1], vec![2], vec![0, 1], vec![1, 2], vec![2, 0], vec![0, 2, 1], vec![1, 0]];
        let mut n = 0;
        for mask in 0u32..(1 << pool.len()) {
            let ballots: Vec<Vec<usize>> = pool.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, b)| b.clone()).collect();
            let t = count_ranked(&ballots, &[0, 1, 2]);
            match &t.outcome {
                Outcome::Winner { option } => {
                    let last = t.rounds.last().unwrap();
                    let active: u32 = last.counts.iter().map(|c| c.1).sum();
                    let mine = last.counts.iter().find(|c| c.0 == *option).unwrap().1;
                    assert!(2 * mine > active, "{ballots:?}");
                }
                Outcome::Tie { options } => assert!(options.len() >= 2, "{ballots:?}"),
                Outcome::NoVotes => assert!(ballots.is_empty()),
            }
            n += 1;
        }
        assert_eq!(n, 256);
    }

    #[test]
    fn specs_are_checked_in_plain_words() {
        let now = Utc::now();
        let close = (now + Duration::days(5)).to_rfc3339();
        let ok = json!({ "face": "rsvp", "title": "Dinner", "closes_at": close, "event_at": (now + Duration::days(7)).to_rfc3339(), "fields": ["plus_ones", "food_needs"], "price": "€35" });
        assert_eq!(validate(&ok, now), Ok(()));
        let with = |k: &str, v: Value| {
            let mut s = ok.clone();
            s[k] = v;
            validate(&s, now).unwrap_err()
        };
        assert!(with("face", json!("poll")).contains("face must be"));
        assert!(with("color", json!("red")).starts_with("color isn't something"));
        assert!(with("when", json!("2026-12-12T19:30:00Z")).contains("event_at"), "an RFC 3339 when would start the core's lobby clock");
        assert!(with("closes_at", json!((now - Duration::hours(1)).to_rfc3339())).contains("already past"));
        assert!(with("event_at", json!((now + Duration::days(1)).to_rfc3339())).contains("before closes_at"));
        assert!(with("fields", json!(["slot_note"])).contains("can add: plus_ones, food_needs"));
        assert!(with("slots", json!([{ "name": "x", "cap": 1 }])).contains("for a sign-up"));
        assert!(with("templates", json!({ "group": "hi" })).contains("isn't a nudge"));
        assert!(with("members", json!(["Ana", ""])).contains("in members"));
        let vote = json!({ "face": "vote", "title": "Where?", "closes_at": close, "ballot": { "kind": "ranked", "options": ["A", "B"] } });
        assert_eq!(validate(&vote, now), Ok(()));
        let mut v = vote.clone();
        v["ballot"]["options"] = json!(["A", "a"]);
        assert!(validate(&v, now).unwrap_err().contains("twice"));
        v["ballot"]["options"] = json!(["A"]);
        assert!(validate(&v, now).unwrap_err().contains("2 to 10"));
        v["price"] = json!("€5");
        assert!(validate(&v, now).is_err());
        let signup = json!({ "face": "signup", "title": "Potluck", "closes_at": close, "slots": [{ "name": "Mains", "cap": 3 }, { "name": "mains", "cap": 1 }] });
        assert!(validate(&signup, now).unwrap_err().contains("Two slots"));
        assert!(validate(&json!({ "face": "signup", "title": "P", "closes_at": close, "slots": [{ "name": "Mains", "cap": 0 }] }), now).unwrap_err().contains("cap"));
    }

    #[test]
    fn retention_is_seven_days_after_the_event_else_the_close() {
        let spec = json!({ "event_at": "2026-12-12T19:30:00Z", "closes_at": "2026-12-10T20:00:00Z" });
        let r = Gather.rules(&spec).retention;
        assert_eq!(r.expires_at(&spec, None).unwrap().to_rfc3339(), "2026-12-19T19:30:00+00:00");
        let spec = json!({ "closes_at": "2026-12-02T20:00:00Z" });
        assert_eq!(r.expires_at(&spec, None).unwrap().to_rfc3339(), "2026-12-09T20:00:00+00:00");
    }

    #[test]
    fn drafts_fill_the_template_and_keep_the_link_for_the_card() {
        let spec = json!({ "face": "rsvp", "title": "the club dinner", "when": "Sat", "place": "The Old Mill", "host_name": "Oleks", "templates": { "maybe": "Hi {name}! Still a maybe for {title} on {when} at {place}? Tap to change it: {link}" } });
        assert_eq!(fill(&spec, "maybe", "Leo", None), "Hi Leo! Still a maybe for the club dinner on Sat at The Old Mill? Tap to change it: {link}");
        assert!(fill(&spec, "not_paid", "Ana", None).contains("Pay Oleks"), "the default when there's no template");
    }
}
