//! The pocket toolkit (pa12 L3): the pure pieces more than one kit plays with, in one place so a
//! fix lands everywhere. Moved here unchanged from the kits that first needed them:
//! - the draw: [`Rng`], [`draw_loop`] and [`insert_into_loop`] (from the Secret Santa kit, `santa.rs`);
//! - the vote counts: [`count_ranked`], [`count_one_pick`] and the [`Tally`] they return (from the
//!   gather kit, `gather.rs`);
//! - text answers: [`match_text`] and [`normalise`], which stay defined in the quiz kit's
//!   `kits/quiz/matching.rs` and are re-exported here, so there is still one definition.
//!
//! The old paths (`santa::draw_loop`, `gather::count_ranked`, ...) re-export these, so callers
//! don't change. Nothing here touches a pocket, the database or the clock.

use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::kits::quiz::matching::{match_text, normalise, TextMatch};
/// The largest +/- a host gives in one tap: the quiz's cap, re-exported for the made kit (pa12 L4).
pub use crate::kits::quiz::engine::MAX_ADJUST;

/// Fewest people a draw takes.
pub const MIN_PEOPLE: usize = 3;

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

pub(crate) fn round_counts(ballots: &[Vec<usize>], inn: &BTreeSet<usize>) -> (BTreeMap<usize, u32>, u32) {
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
