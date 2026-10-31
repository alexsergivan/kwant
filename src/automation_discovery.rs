//! Automation Discovery's miner (Mission Control Phase D4): which repeated board tasks look like a
//! habit worth a routine, and what that routine would be. Deterministic and pure: no model, no
//! clock, no database, so tests pin every instant, daylight-saving changes included. The server
//! loads the rows (`db::automation`) and decides what to do with the result. Design:
//! mission-control/phase-d/automation-discovery.md §4–§5; contract: docs/mission-control.md §20.

use crate::routines::{now_in, run_time_text, MonthDay, Schedule};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Timelike, Utc, Weekday};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

/// How far back daily, weekday and weekly habits are read.
pub const WINDOW_DAYS: i64 = 60;
/// Monthly habits need longer: three months span at least 59 days.
pub const MONTHLY_WINDOW_DAYS: i64 = 120;
/// Share of a pattern's slots, first to last, that had an ask: one skipped week in four passes.
pub const MIN_COVERAGE: f64 = 0.75;
/// Share of the asks that fit the pattern: one off-pattern ask in five passes.
pub const MIN_PRECISION: f64 = 0.80;
/// A habit is stopped once this many of its slots in a row have passed without an ask.
pub const STOPPED_AFTER_MISSED: usize = 2;
pub const MIN_VERIFIED: usize = 2;
/// Asks whose times lie further apart than this are offered with "time varies".
pub const TIME_VARIES_MINUTES: u32 = 4 * 60;
/// Word overlap at which a routine's name already covers a habit.
pub const COVERED_JACCARD: f64 = 0.6;
pub const DISMISS_COOL_OFF_DAYS: i64 = 30;
pub const DEFAULT_DAILY_CAP: i64 = 2;
pub const DAILY_CAP_RANGE: (i64, i64) = (0, 10);
pub const MAX_LIVE_PER_SPACE: usize = 5;
/// A follow-up joins the routine when it came after at least this many asks, and this share.
pub const CHAIN_MIN: usize = 3;
pub const CHAIN_SHARE: f64 = 0.80;
pub const CHAIN_MAX_STEPS: usize = 2;
/// Most recent asks a chain is looked for after.
pub const CHAIN_MAX_SOURCES: usize = 20;
/// How long after an ask is done its follow-up may come.
pub const CHAIN_HOURS: i64 = 24;
/// What a routine may carry (as `POST /api/routines` allows).
pub const MAX_CHECKS: usize = 10;
/// How far a habit's usual time must move before its live offer is replaced.
pub const SUPERSEDE_MINUTES: i64 = 30;
const NAME_CHARS: usize = 80;
const SOURCES_IN_INSTRUCTIONS: usize = 5;

/// Words that say when, not what: dropped from titles before they are compared.
const WHEN_WORDS: &[&str] = &[
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
    "mon", "tue", "tues", "wed", "thu", "thur", "thurs", "fri", "sat", "sun",
    "montag", "dienstag", "mittwoch", "donnerstag", "freitag", "samstag", "sonnabend", "sonntag",
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
    "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
    "januar", "jänner", "februar", "märz", "mai", "juni", "juli", "oktober", "dezember", "okt", "dez",
    "today", "tomorrow", "heute", "morgen",
];

// Titles

/// A title reduced to what was asked: NFKC, lowercase, without dates, numbers, ordinals,
/// weekday and month names (English and German), "today"/"tomorrow", or punctuation.
pub fn norm(title: &str) -> String {
    let text: String = title.nfkc().collect::<String>().to_lowercase();
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().any(|c| c.is_numeric()) && !WHEN_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Too little to be a habit ("fix", "todo"): fewer than 2 words and fewer than 8 letters.
fn too_vague(normed: &str) -> bool {
    normed.split(' ').filter(|w| !w.is_empty()).count() < 2 && normed.chars().filter(|c| c.is_alphabetic()).count() < 8
}

/// The habit a task belongs to: `class:<class>` for a Flight School class other than
/// `general`, else `title:<hash of norm(title)>`. `None` when the title is too vague.
pub fn cluster_key(task_class: &str, title: &str) -> Option<String> {
    let class = task_class.trim();
    if !class.is_empty() && class != "general" {
        return Some(format!("class:{class}"));
    }
    let normed = norm(title);
    if normed.is_empty() || too_vague(&normed) {
        return None;
    }
    let hash = Sha256::digest(normed.as_bytes());
    Some(format!("title:{}", hash.iter().take(6).map(|b| format!("{b:02x}")).collect::<String>()))
}

fn words(s: &str) -> HashSet<String> {
    norm(s).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Word overlap of two titles after [`norm`]: shared words over all words.
pub fn jaccard(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / union as f64
}

/// A title without its date and number words, for a routine's name.
pub fn routine_name(title: &str) -> String {
    let kept: Vec<&str> = title.split_whitespace().filter(|w| !w.chars().any(|c| c.is_numeric())).collect();
    let name = kept.join(" ");
    let name = name.trim_matches(|c: char| !c.is_alphanumeric() && c != ')' && c != '"' && c != '\'');
    let name = if name.is_empty() { title.trim() } else { name };
    if name.chars().count() <= NAME_CHARS {
        name.to_string()
    } else {
        name.chars().take(NAME_CHARS).collect::<String>().trim_end().to_string()
    }
}

// What the miner reads

/// A board task the person created (see `Database::discovery_tasks` for the filter).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub acceptance: String,
    pub state: String,
    pub owner_kwant_id: Option<String>,
    pub task_class: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A check that passed on one of those tasks. `spec` is its JSON text.
#[derive(Debug, Clone, PartialEq)]
pub struct PassedCheck {
    pub task_id: String,
    pub kind: String,
    pub spec: String,
    pub required: bool,
}

/// An existing routine of the space, on or off.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineRef {
    pub kwant_id: String,
    pub name: String,
    pub instructions: String,
    pub task_class: Option<String>,
    /// The habit it was accepted from, for a discovered routine.
    pub cluster_key: Option<String>,
}

/// What earlier offers in the space say about this scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Past {
    /// Habits dismissed, or whose discovered routine was deleted, still cooling off.
    pub cooling: HashSet<String>,
    /// Habits with a live offer.
    pub live: HashSet<String>,
    /// Follow-ups written into a live offer's instructions.
    pub live_chain: HashSet<String>,
    /// Offers made in the last 24 hours.
    pub offered_24h: usize,
    /// Live offers now.
    pub live_count: usize,
}

pub struct Input<'a> {
    /// The person's time zone (IANA).
    pub tz: &'a str,
    pub now: DateTime<Utc>,
    pub tasks: &'a [TaskRow],
    pub checks: &'a [PassedCheck],
    pub routines: &'a [RoutineRef],
    /// Bots that may run a routine here: in the space.
    pub bots: &'a HashSet<String>,
    pub past: &'a Past,
    pub daily_cap: usize,
    pub max_live: usize,
}

// What it finds

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    Monthly,
    Weekly,
    Weekdays,
    Daily,
}

/// Why a habit gets no offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Too few asks, or over too short a time, to tell.
    TooShort,
    /// No daily, weekday, weekly or monthly rhythm.
    NotRegular,
    /// It was regular, then two of its slots in a row passed without an ask.
    Stopped,
    /// Fewer than 2, or under 2/3, of its tasks were verified.
    NotVerified,
    /// No one bot did at least 2/3 of the verified ones.
    MixedOwners,
    /// The bot that did them isn't in the space any more.
    BotUnavailable,
    /// A routine, on or off, already does it.
    Covered,
    /// Dismissed, or its discovered routine deleted, within 30 days.
    CoolingOff,
    /// It already has a live offer.
    AlreadyOffered,
    /// It is a follow-up step of another offer.
    InChain,
    /// Over the daily cap, or the space has 5 live offers.
    Capped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skip {
    pub cluster_key: String,
    pub title: String,
    pub asks: usize,
    pub reason: Reason,
}

/// Where the routine would post.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Delivery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_kwant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckSpec {
    pub kind: String,
    pub spec: Value,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeftOut {
    pub kind: String,
    pub spec: Value,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChainStep {
    pub key: String,
    pub title: String,
    pub kwant_id: Option<String>,
    /// Same bot: a step of the routine's instructions. Another bot: shown only.
    pub in_instructions: bool,
    /// After how many of the asks it came.
    pub after: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Source {
    pub task_id: String,
    pub title: String,
    pub local_date: NaiveDate,
    pub local_time: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evidence {
    pub pattern: Pattern,
    pub matched: usize,
    pub expected: usize,
    pub coverage: f64,
    pub precision: f64,
    pub verified: usize,
    /// Verified plus failed: the tasks that have an outcome.
    pub settled: usize,
    /// Asks in the window, one per local day.
    pub total: usize,
    pub time_spread: String,
    pub time_varies: bool,
    /// The newest matched ask: Accept recomputes the first run from it (design §5).
    pub last_ask_at: DateTime<Utc>,
}

/// A routine the miner would suggest. Nothing here is stored or created by the miner.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Offer {
    pub cluster_key: String,
    pub kwant_id: String,
    pub name: String,
    pub instructions: String,
    pub schedule: Schedule,
    pub first_run_at: DateTime<Utc>,
    pub first_run_text: String,
    pub delivery: Delivery,
    pub task_class: String,
    pub acceptance: String,
    pub checks: Vec<CheckSpec>,
    pub left_out_checks: Vec<LeftOut>,
    pub chain: Vec<ChainStep>,
    pub source_task_ids: Vec<String>,
    pub sources: Vec<Source>,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Scan {
    pub offered: Vec<Offer>,
    pub skipped: Vec<Skip>,
    /// Habits that already have a live offer and still pass, as they would be offered now: the
    /// caller keeps, or replaces, the live offer with these and expires the rest (design §6).
    /// Each is also in `skipped` as `already_offered`.
    #[serde(skip)]
    pub live: Vec<Offer>,
}

// Cadence

/// One local day with an ask: the earliest task of that day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    /// Local minutes since midnight.
    pub minutes: u32,
    /// Index into the cluster's tasks.
    pub task: usize,
}

/// A rhythm that fits: which days matched which slot, and how well.
#[derive(Debug, Clone, PartialEq)]
pub struct Cadence {
    pub pattern: Pattern,
    /// Weekly: the weekdays, Monday first.
    pub weekdays: Vec<Weekday>,
    pub month_day: Option<MonthDay>,
    /// Indices into the days given to [`detect`].
    pub matched: Vec<usize>,
    pub expected: usize,
    pub coverage: f64,
    pub precision: f64,
}

/// How one pattern fared against the days.
#[derive(Debug, Clone)]
struct Fit {
    matched: Vec<usize>,
    slots: Vec<NaiveDate>,
    expected: usize,
    coverage: f64,
    precision: f64,
    exact: usize,
    missed_since: usize,
}

/// Matches `days` (sorted, one per date) to the dates `is_slot` accepts. An ask on a slot matches
/// it; an ask off the slots matches the nearest slot within `shift` days that no ask is on. Each
/// slot takes one ask. `today` counts the slots passed since the last match.
fn fit(days: &[Day], is_slot: &dyn Fn(NaiveDate) -> bool, shift: i64, today: NaiveDate) -> Fit {
    let mut taken: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    let mut exact = 0;
    for (i, d) in days.iter().enumerate() {
        if is_slot(d.date) {
            taken.insert(d.date, i);
            exact += 1;
        }
    }
    for (i, d) in days.iter().enumerate() {
        if is_slot(d.date) {
            continue;
        }
        let near = (1..=shift)
            .flat_map(|n| [d.date - Duration::days(n), d.date + Duration::days(n)])
            .find(|s| is_slot(*s) && !taken.contains_key(s));
        if let Some(s) = near {
            taken.insert(s, i);
        }
    }
    let (Some(first), Some(last)) = (taken.keys().next().copied(), taken.keys().next_back().copied()) else {
        return Fit { matched: vec![], slots: vec![], expected: 0, coverage: 0.0, precision: 0.0, exact: 0, missed_since: 0 };
    };
    let expected = first.iter_days().take_while(|d| *d <= last).filter(|d| is_slot(*d)).count();
    // Slots after the last match whose time to be done in (with the shift) has passed
    let missed_since = last
        .iter_days()
        .skip(1)
        .take_while(|d| *d + Duration::days(shift) < today)
        .filter(|d| is_slot(*d))
        .count();
    let mut matched: Vec<usize> = taken.values().copied().collect();
    matched.sort_unstable();
    Fit {
        coverage: taken.len() as f64 / expected.max(1) as f64,
        precision: taken.len() as f64 / days.len().max(1) as f64,
        slots: taken.keys().copied().collect(),
        matched,
        expected,
        exact,
        missed_since,
    }
}

impl Fit {
    fn regular(&self) -> bool {
        self.coverage >= MIN_COVERAGE && self.precision >= MIN_PRECISION
    }

    fn span_days(&self) -> i64 {
        match (self.slots.first(), self.slots.last()) {
            (Some(a), Some(b)) => (*b - *a).num_days(),
            _ => 0,
        }
    }
}

const WEEK: [Weekday; 7] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

fn in_window(days: &[Day], today: NaiveDate, window: i64) -> Vec<Day> {
    days.iter().copied().filter(|d| (today - d.date).num_days() < window).collect()
}

/// Indices into `within` mapped back to indices into `all` (both sorted, `within` a suffix).
fn reindex(all: &[Day], within: &[Day], matched: &[usize]) -> Vec<usize> {
    let offset = all.len() - within.len();
    matched.iter().map(|i| i + offset).collect()
}

/// The first rhythm that fits `days` (sorted by date, one per date, within
/// [`MONTHLY_WINDOW_DAYS`] of `today`), most specific first: monthly, weekly, weekdays, daily.
/// `Err` says why none does.
pub fn detect(days: &[Day], today: NaiveDate) -> Result<Cadence, Reason> {
    if days.len() < 3 {
        return Err(Reason::TooShort);
    }
    let mut why = Reason::NotRegular;
    let mut note = |f: &Fit, enough: bool| -> bool {
        if !f.regular() {
            return false;
        }
        if !enough {
            if why == Reason::NotRegular {
                why = Reason::TooShort;
            }
            return false;
        }
        if f.missed_since >= STOPPED_AFTER_MISSED {
            why = Reason::Stopped;
            return false;
        }
        true
    };

    // Monthly: day N (a short month's last day stands in for N past it), or the last day
    let monthly = in_window(days, today, MONTHLY_WINDOW_DAYS);
    let last_of = |d: NaiveDate| crate_last_day(d);
    let mut best: Option<(MonthDay, Fit)> = None;
    for md in (1..=31).map(MonthDay::Day).chain([MonthDay::Last]) {
        let is_slot = move |d: NaiveDate| match md {
            MonthDay::Last => d.day() == last_of(d),
            MonthDay::Day(n) => d.day() == n.min(last_of(d)),
        };
        let f = fit(&monthly, &is_slot, 3, today);
        let better = match &best {
            None => true,
            Some((_, b)) => (f.matched.len(), f.exact) > (b.matched.len(), b.exact),
        };
        if better {
            best = Some((md, f));
        }
    }
    if let Some((md, f)) = best {
        if note(&f, f.matched.len() >= 3) {
            let all_last = f.matched.iter().all(|&i| monthly[i].date.day() == last_of(monthly[i].date));
            let short = f.matched.iter().any(|&i| last_of(monthly[i].date) < 31);
            let md = if all_last && short { MonthDay::Last } else { md };
            return Ok(Cadence {
                pattern: Pattern::Monthly,
                weekdays: vec![],
                month_day: Some(md),
                matched: reindex(days, &monthly, &f.matched),
                expected: f.expected,
                coverage: f.coverage,
                precision: f.precision,
            });
        }
    }

    let recent = in_window(days, today, WINDOW_DAYS);
    if recent.len() < 3 {
        // Older asks that fit no month aren't a habit cut short
        return Err(if why == Reason::NotRegular && monthly.len() < 3 { Reason::TooShort } else { why });
    }

    // Weekly: one weekday, or up to three each asked twice; each must hold its own slots
    let mut counts: Vec<(Weekday, usize)> = WEEK.iter().map(|w| (*w, recent.iter().filter(|d| d.date.weekday() == *w).count())).collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.num_days_from_monday().cmp(&b.0.num_days_from_monday())));
    for k in 1..=3 {
        let set: Vec<Weekday> = counts.iter().take(k).map(|(w, _)| *w).collect();
        if k > 1 && counts[..k].iter().any(|(_, n)| *n < 2) {
            break;
        }
        let f = fit(&recent, &|d: NaiveDate| set.contains(&d.weekday()), 1, today);
        let each_holds = set.iter().all(|w| {
            let mine = f.slots.iter().filter(|s| s.weekday() == *w).count();
            let expected = match (f.slots.first(), f.slots.last()) {
                (Some(a), Some(b)) => a.iter_days().take_while(|d| d <= b).filter(|d| d.weekday() == *w).count(),
                _ => 0,
            };
            mine as f64 >= MIN_COVERAGE * expected as f64
        });
        if !each_holds {
            continue;
        }
        if note(&f, f.matched.len() >= 3 && f.span_days() >= 14) {
            let mut weekdays = set.clone();
            weekdays.sort_by_key(|w| w.num_days_from_monday());
            return Ok(Cadence {
                pattern: Pattern::Weekly,
                weekdays,
                month_day: None,
                matched: reindex(days, &recent, &f.matched),
                expected: f.expected,
                coverage: f.coverage,
                precision: f.precision,
            });
        }
    }

    // Weekdays: Monday to Friday, at least four of them seen
    let workday = |d: NaiveDate| d.weekday().num_days_from_monday() < 5;
    let f = fit(&recent, &workday, 0, today);
    let distinct: HashSet<Weekday> = f.slots.iter().map(|d| d.weekday()).collect();
    if note(&f, f.matched.len() >= 5 && distinct.len() >= 4) {
        return Ok(Cadence {
            pattern: Pattern::Weekdays,
            weekdays: WEEK[..5].to_vec(),
            month_day: None,
            matched: reindex(days, &recent, &f.matched),
            expected: f.expected,
            coverage: f.coverage,
            precision: f.precision,
        });
    }

    // Daily: any day, a weekend day among them
    let f = fit(&recent, &|_| true, 0, today);
    let weekend = f.slots.iter().any(|d| !workday(*d));
    if note(&f, f.matched.len() >= 5 && weekend) {
        return Ok(Cadence {
            pattern: Pattern::Daily,
            weekdays: vec![],
            month_day: None,
            matched: reindex(days, &recent, &f.matched),
            expected: f.expected,
            coverage: f.coverage,
            precision: f.precision,
        });
    }
    Err(why)
}

fn crate_last_day(date: NaiveDate) -> u32 {
    let (y, m) = (date.year(), date.month());
    let first_next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m + 1, 1) };
    first_next.and_then(|d| d.pred_opt()).map_or(28, |d| d.day())
}

fn weekday_name(w: Weekday) -> &'static str {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"][w.num_days_from_monday() as usize]
}

fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The usual time of the matched asks: their median, down to the quarter hour.
pub fn usual_time(minutes: &[u32]) -> u32 {
    let mut m = minutes.to_vec();
    m.sort_unstable();
    let median = match m.len() {
        0 => 9 * 60,
        n if n % 2 == 1 => m[n / 2],
        n => (m[n / 2 - 1] + m[n / 2]) / 2,
    };
    median - median % 15
}

/// The routine schedule for a cadence at `minutes` local time in `tz`.
pub fn schedule_for(c: &Cadence, minutes: u32, tz: &str) -> Schedule {
    let time = hhmm(minutes);
    let tz = tz.to_string();
    match c.pattern {
        Pattern::Daily => Schedule::Daily { time, tz },
        Pattern::Weekly | Pattern::Weekdays => Schedule::Weekly { time, weekdays: c.weekdays.iter().map(|w| weekday_name(*w).to_string()).collect(), tz },
        Pattern::Monthly => Schedule::Monthly { time, days: vec![c.month_day.unwrap_or(MonthDay::Day(1))], tz },
    }
    .canonical()
}

/// How often a schedule runs, roughly: half of it is how long after an ask its first run waits.
fn period(s: &Schedule) -> Duration {
    match s {
        Schedule::Weekly { weekdays, .. } => Duration::minutes(7 * 24 * 60 / weekdays.len().max(1) as i64),
        Schedule::Monthly { days, .. } => Duration::days(30 / days.len().max(1) as i64),
        Schedule::Interval { every_minutes } => Duration::minutes(*every_minutes),
        _ => Duration::days(1),
    }
}

/// When a suggested routine would first run: the schedule's next run after now, but not within
/// half a period of the last ask, so this period's ask, already done, isn't run again. Accept
/// calls this again with the final schedule (design §5).
pub fn first_run_at(schedule: &Schedule, last_ask: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    schedule.next_after(now.max(last_ask + period(schedule) / 2), None)
}

// The miner

struct Cluster<'a> {
    key: String,
    tasks: Vec<&'a TaskRow>,
}

/// The local date and minute of `at` in `tz`; `None` for an unreadable zone.
fn local(tz: &str, at: DateTime<Utc>) -> Option<(NaiveDate, u32)> {
    let l = now_in(tz, at).ok()?;
    Some((l.date_naive(), l.hour() * 60 + l.minute()))
}

/// The cluster's asks, one per local day (the earliest), oldest first.
fn days_of(c: &Cluster, tz: &str) -> Vec<Day> {
    let mut by_date: BTreeMap<NaiveDate, Day> = BTreeMap::new();
    for (i, t) in c.tasks.iter().enumerate() {
        let Some((date, minutes)) = local(tz, t.created_at) else { continue };
        let day = Day { date, minutes, task: i };
        by_date
            .entry(date)
            .and_modify(|d| {
                if t.created_at < c.tasks[d.task].created_at {
                    *d = day;
                }
            })
            .or_insert(day);
    }
    by_date.into_values().collect()
}

fn most_common<T: Eq + std::hash::Hash + Clone>(items: impl IntoIterator<Item = T>) -> Option<(T, usize)> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(t, _)| *t == item) {
            Some((_, n)) => *n += 1,
            None => counts.push((item, 1)),
        }
    }
    // First seen wins a tie: callers pass newest first
    counts.into_iter().fold(None, |best, (t, n)| match best {
        Some((_, bn)) if bn >= n => best,
        _ => Some((t, n)),
    })
}

/// Checks that passed with the same spec on at least two of the verified asks; the rest are
/// listed with why they were left out.
fn checks_of(verified: &[&TaskRow], passed: &[PassedCheck]) -> (Vec<CheckSpec>, Vec<LeftOut>) {
    let ids: Vec<&str> = verified.iter().map(|t| t.id.as_str()).collect();
    // (kind, spec) → tasks it passed on, in the order first seen (newest task first)
    let mut groups: Vec<((String, Value), HashSet<&str>, bool)> = Vec::new();
    for id in &ids {
        for c in passed.iter().filter(|c| c.task_id == *id) {
            let spec: Value = serde_json::from_str(&c.spec).unwrap_or(Value::String(c.spec.clone()));
            let key = (c.kind.clone(), spec);
            match groups.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, tasks, required)) => {
                    tasks.insert(id);
                    *required |= c.required;
                }
                None => groups.push((key, HashSet::from([*id]), c.required)),
            }
        }
    }
    let (mut keep, mut left) = (Vec::new(), Vec::<LeftOut>::new());
    for ((kind, spec), tasks, required) in &groups {
        let reason = if matches!(kind.as_str(), "person" | "peer_review") {
            Some("needs a person or a reviewer")
        } else if tasks.len() >= 2 {
            None
        } else if groups.iter().any(|((k, s), t, _)| k == kind && s != spec && !t.is_subset(tasks)) {
            Some("changes every time")
        } else {
            Some("passed only once")
        };
        match reason {
            None if keep.len() < MAX_CHECKS => keep.push(CheckSpec { kind: kind.clone(), spec: spec.clone(), required: *required }),
            None => left.push(LeftOut { kind: kind.clone(), spec: spec.clone(), reason: "more than 10 checks" }),
            // One line per kind and reason is enough to explain it
            Some(r) if !left.iter().any(|l| l.kind == *kind && l.reason == r) => left.push(LeftOut { kind: kind.clone(), spec: spec.clone(), reason: r }),
            Some(_) => {}
        }
    }
    (keep, left)
}

/// Follow-ups the person asks for after this habit's asks, in up to [`CHAIN_MAX_STEPS`].
fn chain_of(key: &str, asks: &[&TaskRow], all: &[(Option<String>, &TaskRow)], bot: &str) -> Vec<ChainStep> {
    let asks: Vec<&TaskRow> = asks.iter().rev().take(CHAIN_MAX_SOURCES).copied().collect();
    // key → (asks it followed, its tasks newest first, minutes after the ask)
    let mut follows: HashMap<&str, (usize, Vec<&TaskRow>, Vec<i64>)> = HashMap::new();
    for a in &asks {
        let until = a.finished_at.unwrap_or(a.created_at) + Duration::hours(CHAIN_HOURS);
        let mut seen: HashSet<&str> = HashSet::new();
        for (k, b) in all {
            let Some(k) = k.as_deref().filter(|k| *k != key) else { continue };
            if b.created_at <= a.created_at || b.created_at > until || b.id == a.id {
                continue;
            }
            let e = follows.entry(k).or_default();
            if seen.insert(k) {
                e.0 += 1;
                e.2.push((b.created_at - a.created_at).num_minutes());
            }
            e.1.push(b);
        }
    }
    let mut steps: Vec<(&str, usize, i64, &Vec<&TaskRow>)> = follows
        .iter()
        .filter(|(_, (n, _, _))| *n >= CHAIN_MIN && *n as f64 >= CHAIN_SHARE * asks.len() as f64)
        .map(|(k, (n, tasks, delays))| {
            let mut d = delays.clone();
            d.sort_unstable();
            (*k, *n, d[d.len() / 2], tasks)
        })
        .collect();
    steps.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(a.0.cmp(b.0)));
    steps
        .into_iter()
        .take(CHAIN_MAX_STEPS)
        .map(|(k, n, _, tasks)| {
            let mut newest = tasks.clone();
            newest.sort_by_key(|t| std::cmp::Reverse(t.created_at));
            let owner = most_common(newest.iter().filter_map(|t| t.owner_kwant_id.clone())).map(|(o, _)| o);
            ChainStep {
                key: k.to_string(),
                title: newest.first().map(|t| t.title.trim().to_string()).unwrap_or_default(),
                in_instructions: owner.as_deref() == Some(bot),
                kwant_id: owner,
                after: n,
            }
        })
        .collect()
}

fn instructions(name: &str, acceptance: &str, chain: &[ChainStep], sources: &[Source]) -> String {
    let mut out = name.to_string();
    if !acceptance.trim().is_empty() {
        out.push_str(&format!("\n\nA good result: {}", acceptance.trim()));
    }
    let steps: Vec<&ChainStep> = chain.iter().filter(|s| s.in_instructions).collect();
    if !steps.is_empty() {
        out.push_str("\n\nThen, in the same run:");
        for (i, s) in steps.iter().enumerate() {
            out.push_str(&format!("\n{}. {}", i + 1, s.title));
        }
    }
    let past: Vec<String> = sources.iter().rev().take(SOURCES_IN_INSTRUCTIONS).map(|s| format!("\"{}\" ({})", s.title, s.local_date)).collect();
    out.push_str(&format!("\n\nBased on your tasks: {}", past.join(", ")));
    out
}

/// Whether an existing routine already does this habit for this bot.
fn covered(key: &str, class: &str, bot: &str, title: &str, routines: &[RoutineRef]) -> bool {
    routines.iter().any(|r| {
        r.cluster_key.as_deref() == Some(key)
            || (r.kwant_id == bot
                && ((class != "general" && r.task_class.as_deref() == Some(class))
                    || jaccard(&r.name, title) >= COVERED_JACCARD
                    || jaccard(r.instructions.lines().next().unwrap_or(""), title) >= COVERED_JACCARD))
    })
}

/// One scan of a space: the offers it would make and why every other habit gets none.
/// Reads only its input; the caller decides whether to store anything.
pub fn mine(input: &Input) -> Scan {
    let Some((today, _)) = local(input.tz, input.now) else { return Scan::default() };
    let keyed: Vec<(Option<String>, &TaskRow)> = input.tasks.iter().map(|t| (cluster_key(&t.task_class, &t.title), t)).collect();
    let mut clusters: Vec<Cluster> = Vec::new();
    for (k, t) in &keyed {
        let Some(k) = k else { continue };
        match clusters.iter_mut().find(|c| c.key == *k) {
            Some(c) => c.tasks.push(t),
            None => clusters.push(Cluster { key: k.clone(), tasks: vec![t] }),
        }
    }
    let mut passing: Vec<Offer> = Vec::new();
    let mut skipped: Vec<Skip> = Vec::new();
    let mut live: Vec<Offer> = Vec::new();
    for c in &mut clusters {
        c.tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let days = days_of(c, input.tz);
        let newest_title = c.tasks.last().map(|t| t.title.trim().to_string()).unwrap_or_default();
        let skip = |reason| Skip { cluster_key: c.key.clone(), title: newest_title.clone(), asks: days.len(), reason };
        let cadence = match detect(&days, today) {
            Ok(cd) => cd,
            Err(reason) => {
                skipped.push(skip(reason));
                continue;
            }
        };
        let asks: Vec<&TaskRow> = cadence.matched.iter().map(|&i| c.tasks[days[i].task]).collect();
        let verified: Vec<&TaskRow> = asks.iter().rev().filter(|t| t.state == "verified").copied().collect();
        let settled = verified.len() + asks.iter().filter(|t| t.state == "failed").count();
        if verified.len() < MIN_VERIFIED || verified.len() * 3 < settled * 2 {
            skipped.push(skip(Reason::NotVerified));
            continue;
        }
        let Some((bot, owned)) = most_common(verified.iter().filter_map(|t| t.owner_kwant_id.clone())) else {
            skipped.push(skip(Reason::MixedOwners));
            continue;
        };
        if owned * 3 < verified.len() * 2 {
            skipped.push(skip(Reason::MixedOwners));
            continue;
        }
        if !input.bots.contains(&bot) {
            skipped.push(skip(Reason::BotUnavailable));
            continue;
        }
        let latest = verified[0];
        let class = if c.key.starts_with("class:") { latest.task_class.trim().to_string() } else { "general".to_string() };
        if covered(&c.key, &class, &bot, &latest.title, input.routines) {
            skipped.push(skip(Reason::Covered));
            continue;
        }
        if input.past.cooling.contains(&c.key) {
            skipped.push(skip(Reason::CoolingOff));
            continue;
        }
        let is_live = input.past.live.contains(&c.key);
        if is_live {
            skipped.push(skip(Reason::AlreadyOffered));
        }

        let minutes: Vec<u32> = cadence.matched.iter().map(|&i| days[i].minutes).collect();
        let (lo, hi) = (minutes.iter().copied().min().unwrap_or(0), minutes.iter().copied().max().unwrap_or(0));
        let schedule = schedule_for(&cadence, usual_time(&minutes), input.tz);
        let last_ask = asks.last().map_or(input.now, |t| t.created_at);
        let Some(first_run_at) = first_run_at(&schedule, last_ask, input.now) else {
            skipped.push(skip(Reason::NotRegular));
            continue;
        };
        let name = routine_name(&latest.title);
        let acceptance = latest.acceptance.trim().to_string();
        let (checks, left_out_checks) = checks_of(&verified, input.checks);
        let chain = chain_of(&c.key, &asks, &keyed, &bot);
        let sources: Vec<Source> = cadence
            .matched
            .iter()
            .map(|&i| {
                let t = c.tasks[days[i].task];
                Source { task_id: t.id.clone(), title: t.title.trim().to_string(), local_date: days[i].date, local_time: hhmm(days[i].minutes), state: t.state.clone() }
            })
            .collect();
        let newest_first: Vec<&TaskRow> = asks.iter().rev().copied().collect();
        let place = most_common(newest_first.iter().map(|t| (t.thread_id.clone(), t.channel_id.clone()))).map(|(p, _)| p).unwrap_or_default();
        let delivery = match place {
            (Some(thread), _) => Delivery { thread_id: Some(thread), ..Default::default() },
            (None, Some(channel)) => Delivery { channel_id: Some(channel), ..Default::default() },
            (None, None) => Delivery { direct_kwant_id: Some(bot.clone()), ..Default::default() },
        };
        let offer = Offer {
            cluster_key: c.key.clone(),
            instructions: instructions(&name, &acceptance, &chain, &sources),
            first_run_text: run_time_text(first_run_at, schedule.tz()),
            kwant_id: bot,
            name,
            schedule,
            first_run_at,
            delivery,
            task_class: class,
            acceptance,
            checks,
            left_out_checks,
            chain,
            source_task_ids: sources.iter().map(|s| s.task_id.clone()).collect(),
            evidence: Evidence {
                pattern: cadence.pattern,
                matched: cadence.matched.len(),
                expected: cadence.expected,
                coverage: (cadence.coverage * 100.0).round() / 100.0,
                precision: (cadence.precision * 100.0).round() / 100.0,
                verified: verified.len(),
                settled,
                total: days.len(),
                time_spread: if lo == hi { hhmm(lo) } else { format!("{}–{}", hhmm(lo), hhmm(hi)) },
                time_varies: hi - lo > TIME_VARIES_MINUTES,
                last_ask_at: last_ask,
            },
            sources,
        };
        if is_live {
            live.push(offer);
        } else {
            passing.push(offer);
        }
    }

    // A follow-up written into another offer's steps isn't offered on its own
    let steps: HashSet<String> = passing
        .iter()
        .flat_map(|o| o.chain.iter().filter(|s| s.in_instructions).map(|s| s.key.clone()))
        .chain(input.past.live_chain.iter().cloned())
        .collect();
    let (in_chain, mut passing): (Vec<Offer>, Vec<Offer>) = passing.into_iter().partition(|o| steps.contains(&o.cluster_key));
    skipped.extend(in_chain.into_iter().map(|o| Skip { title: o.name.clone(), asks: o.evidence.total, cluster_key: o.cluster_key, reason: Reason::InChain }));

    // The strongest habits first, then the cap
    passing.sort_by(|a, b| {
        b.evidence.matched.cmp(&a.evidence.matched).then(b.sources.last().map(|s| s.local_date).cmp(&a.sources.last().map(|s| s.local_date))).then(a.cluster_key.cmp(&b.cluster_key))
    });
    let room = input.daily_cap.saturating_sub(input.past.offered_24h).min(input.max_live.saturating_sub(input.past.live_count));
    let capped = passing.split_off(room.min(passing.len()));
    skipped.extend(capped.into_iter().map(|o| Skip { title: o.name.clone(), asks: o.evidence.total, cluster_key: o.cluster_key, reason: Reason::Capped }));
    skipped.sort_by(|a, b| a.cluster_key.cmp(&b.cluster_key));
    Scan { offered: passing, skipped, live }
}

/// Whether a live offer (`schedule`, `kwant_id`) no longer says what the habit is now, so a scan
/// replaces it: another rhythm, days, zone or bot, or a usual time moved by at least
/// [`SUPERSEDE_MINUTES`]. Smaller drifts of the median keep the offer, so it doesn't churn.
pub fn materially_changed(schedule: &Schedule, kwant_id: &str, now: &Offer) -> bool {
    if kwant_id != now.kwant_id {
        return true;
    }
    let minutes = |t: &str| NaiveTime::parse_from_str(t, "%H:%M").ok().map(|t| i64::from(t.hour() * 60 + t.minute()));
    let moved = |a: &str, b: &str| match (minutes(a), minutes(b)) {
        (Some(a), Some(b)) => (a - b).abs() >= SUPERSEDE_MINUTES,
        _ => a != b,
    };
    match (schedule, &now.schedule) {
        (Schedule::Daily { time: a, tz: x }, Schedule::Daily { time: b, tz: y }) => x != y || moved(a, b),
        (Schedule::Weekly { time: a, weekdays: d, tz: x }, Schedule::Weekly { time: b, weekdays: e, tz: y }) => d != e || x != y || moved(a, b),
        (Schedule::Monthly { time: a, days: d, tz: x }, Schedule::Monthly { time: b, days: e, tz: y }) => d != e || x != y || moved(a, b),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::local_instant;

    const BERLIN: &str = "Europe/Berlin";

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn at(tz: &str, d: NaiveDate, time: &str) -> DateTime<Utc> {
        local_instant(tz, d, NaiveTime::parse_from_str(time, "%H:%M").unwrap()).unwrap().unwrap()
    }

    /// `n` dates from `start`, `step` days apart.
    fn every(start: &str, step: i64, n: usize) -> Vec<NaiveDate> {
        (0..n).map(|i| date(start) + Duration::days(step * i as i64)).collect()
    }

    fn task(id: &str, title: &str, created_at: DateTime<Utc>) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: title.into(),
            acceptance: "The report is in the shared folder".into(),
            state: "verified".into(),
            owner_kwant_id: Some("bot-a".into()),
            task_class: "general".into(),
            channel_id: None,
            direct_kwant_id: None,
            thread_id: None,
            created_at,
            finished_at: Some(created_at + Duration::minutes(30)),
        }
    }

    /// One verified ask by bot-a per date, at `time` local, titled with its date.
    fn asks(tz: &str, prefix: &str, title: &str, dates: &[NaiveDate], time: &str) -> Vec<TaskRow> {
        dates.iter().enumerate().map(|(i, d)| task(&format!("{prefix}-{i}"), &format!("{title} {d}"), at(tz, *d, time))).collect()
    }

    struct Fx {
        tz: &'static str,
        now: DateTime<Utc>,
        tasks: Vec<TaskRow>,
        checks: Vec<PassedCheck>,
        routines: Vec<RoutineRef>,
        bots: HashSet<String>,
        past: Past,
        cap: usize,
    }

    impl Fx {
        fn new(tz: &'static str, now: DateTime<Utc>, tasks: Vec<TaskRow>) -> Self {
            Fx {
                tz,
                now,
                tasks,
                checks: vec![],
                routines: vec![],
                bots: HashSet::from(["bot-a".to_string(), "bot-b".to_string()]),
                past: Past::default(),
                cap: DEFAULT_DAILY_CAP as usize,
            }
        }

        fn scan(&self) -> Scan {
            mine(&Input {
                tz: self.tz,
                now: self.now,
                tasks: &self.tasks,
                checks: &self.checks,
                routines: &self.routines,
                bots: &self.bots,
                past: &self.past,
                daily_cap: self.cap,
                max_live: MAX_LIVE_PER_SPACE,
            })
        }

        fn only_offer(&self) -> Offer {
            let scan = self.scan();
            assert_eq!(scan.offered.len(), 1, "skipped: {:?}", scan.skipped);
            scan.offered.into_iter().next().unwrap()
        }

        fn only_skip(&self) -> Reason {
            let scan = self.scan();
            assert!(scan.offered.is_empty(), "offered: {:?}", scan.offered.iter().map(|o| &o.name).collect::<Vec<_>>());
            assert_eq!(scan.skipped.len(), 1, "{:?}", scan.skipped);
            scan.skipped[0].reason
        }
    }

    fn days(dates: &[NaiveDate]) -> Vec<Day> {
        dates.iter().enumerate().map(|(i, d)| Day { date: *d, minutes: 9 * 60, task: i }).collect()
    }

    fn local_hm(tz: &str, at: DateTime<Utc>) -> String {
        now_in(tz, at).unwrap().format("%a %H:%M").to_string()
    }

    // Titles

    #[test]
    fn norm_drops_dates_numbers_weekdays_and_months_in_english_and_german() {
        assert_eq!(norm("Weekly report — Mon 3 March 2026"), "weekly report");
        assert_eq!(norm("weekly REPORT (2026-10-05)"), "weekly report");
        assert_eq!(norm("Send invoices for 31st Oct, today"), "send invoices for");
        assert_eq!(norm("Wochenbericht für Montag, 3. März"), "wochenbericht für");
        assert_eq!(norm("Rechnungen schicken: Dezember / heute"), "rechnungen schicken");
        // NFKC: full-width letters and ligatures read like plain ones
        assert_eq!(norm("Ｗｅｅｋｌｙ report"), "weekly report");
        assert_eq!(norm("ﬁle backups"), "file backups");
    }

    #[test]
    fn cluster_key_groups_titles_by_what_was_asked_or_by_class() {
        let a = cluster_key("general", "Weekly report 2026-10-05").unwrap();
        assert_eq!(cluster_key("", "weekly report (Oct 12)"), Some(a.clone()));
        assert!(a.starts_with("title:") && a.len() == "title:".len() + 12, "{a}");
        assert_ne!(cluster_key("general", "Monthly report"), Some(a));
        assert_eq!(cluster_key("invoice", "anything at all"), Some("class:invoice".into()));
        // Too vague to be a habit
        assert_eq!(cluster_key("general", "Fix"), None);
        assert_eq!(cluster_key("general", "todo 3 Oct"), None);
        assert_eq!(cluster_key("general", "Mon 12"), None);
        // One long word is enough
        assert!(cluster_key("general", "Bookkeeping").is_some());
    }

    #[test]
    fn routine_name_drops_date_words_and_is_capped() {
        assert_eq!(routine_name("Weekly sales report 2026-10-05"), "Weekly sales report");
        assert_eq!(routine_name("  Invoices (Oct 3rd) "), "Invoices (Oct");
        assert_eq!(routine_name("2026-10-05"), "2026-10-05");
        assert_eq!(routine_name(&"long ".repeat(40)).chars().count(), 79);
    }

    // Daylight saving: the same local time on both sides of the switch

    #[test]
    fn weekly_across_berlin_spring_forward_keeps_the_local_time() {
        // Mondays 16 Feb to 13 Apr 2026; clocks go forward on Sunday 29 March
        let dates = every("2026-02-16", 7, 9);
        assert!(dates.iter().all(|d| d.weekday() == Weekday::Mon));
        let tasks = asks(BERLIN, "r", "Weekly report", &dates, "09:00");
        // The asks are an hour apart in UTC
        assert_eq!(tasks[0].created_at.hour(), 8);
        assert_eq!(tasks[8].created_at.hour(), 7);
        let fx = Fx::new(BERLIN, at(BERLIN, date("2026-04-14"), "10:00"), tasks);
        let o = fx.only_offer();
        assert_eq!(o.schedule, Schedule::Weekly { time: "09:00".into(), weekdays: vec!["mon".into()], tz: BERLIN.into() });
        assert_eq!(o.evidence.pattern, Pattern::Weekly);
        assert_eq!((o.evidence.matched, o.evidence.expected), (9, 9));
        assert_eq!(o.evidence.time_spread, "09:00");
        assert!(!o.evidence.time_varies);
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-04-20"), "09:00"));
        assert_eq!(o.first_run_text, "Mon 20 Apr 09:00 Europe/Berlin");
        assert_eq!(o.name, "Weekly report");
        assert_eq!(o.kwant_id, "bot-a");
    }

    #[test]
    fn twice_weekly_across_new_york_spring_forward_keeps_the_local_time() {
        // Tuesdays and Thursdays 3 Feb to 26 Mar 2026; clocks go forward on Sunday 8 March
        let dates: Vec<NaiveDate> = every("2026-02-03", 7, 8).into_iter().flat_map(|d| [d, d + Duration::days(2)]).collect();
        let tz = "America/New_York";
        let tasks = asks(tz, "r", "Client status update", &dates, "17:30");
        assert_ne!(tasks[0].created_at.hour(), tasks[15].created_at.hour());
        let fx = Fx::new(tz, at(tz, date("2026-03-27"), "09:00"), tasks);
        let o = fx.only_offer();
        assert_eq!(o.schedule, Schedule::Weekly { time: "17:30".into(), weekdays: vec!["tue".into(), "thu".into()], tz: tz.into() });
        assert_eq!(o.evidence.matched, 16);
        assert_eq!(local_hm(tz, o.first_run_at), "Tue 17:30");
    }

    #[test]
    fn weekly_across_auckland_fall_back_keeps_the_local_day_and_time() {
        // Sundays 1 Mar to 26 Apr 2026 at 08:00, which is Saturday evening in UTC; clocks go
        // back on Sunday 5 April
        let tz = "Pacific/Auckland";
        let dates = every("2026-03-01", 7, 9);
        let tasks = asks(tz, "r", "Plan the week", &dates, "08:00");
        assert_eq!(tasks[0].created_at.weekday(), Weekday::Sat);
        assert_eq!(tasks[0].created_at.hour(), 19);
        assert_eq!(tasks[8].created_at.hour(), 20);
        let fx = Fx::new(tz, at(tz, date("2026-04-27"), "12:00"), tasks);
        let o = fx.only_offer();
        assert_eq!(o.schedule, Schedule::Weekly { time: "08:00".into(), weekdays: vec!["sun".into()], tz: tz.into() });
        assert_eq!(o.first_run_at, at(tz, date("2026-05-03"), "08:00"));
        assert_eq!(o.sources[0].local_date, date("2026-03-01"));
        assert_eq!(o.sources[0].local_time, "08:00");
    }

    // Every cadence kind

    #[test]
    fn daily_needs_every_day_weekends_included() {
        let dates = every("2026-09-12", 1, 21);
        let mut tasks = asks(BERLIN, "d", "Check the inbox", &dates, "07:05");
        tasks[10].created_at += Duration::minutes(20);
        let now = at(BERLIN, date("2026-10-02"), "12:00");
        let o = Fx::new(BERLIN, now, tasks).only_offer();
        assert_eq!(o.schedule, Schedule::Daily { time: "07:00".into(), tz: BERLIN.into() });
        assert_eq!(o.evidence.time_spread, "07:05–07:25");
        // Asked today already: the first run is tomorrow
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-10-03"), "07:00"));
    }

    #[test]
    fn weekdays_is_monday_to_friday_and_a_weekend_skips_no_slot() {
        // Four working weeks, 7 Sep to 2 Oct 2026
        let dates: Vec<NaiveDate> = every("2026-09-07", 1, 26).into_iter().filter(|d| d.weekday().num_days_from_monday() < 5).collect();
        assert_eq!(dates.len(), 20);
        let tasks = asks(BERLIN, "w", "Standup notes", &dates, "08:45");
        // Monday after the last Friday: no slot missed yet
        let o = Fx::new(BERLIN, at(BERLIN, date("2026-10-05"), "07:00"), tasks).only_offer();
        assert_eq!(o.evidence.pattern, Pattern::Weekdays);
        assert_eq!(o.schedule, Schedule::Weekly { time: "08:45".into(), weekdays: ["mon", "tue", "wed", "thu", "fri"].map(String::from).to_vec(), tz: BERLIN.into() });
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-10-05"), "08:45"));
    }

    #[test]
    fn monthly_on_a_day_of_the_month() {
        let dates = ["2026-07-03", "2026-08-03", "2026-09-03", "2026-10-03"].map(date);
        let tasks = asks(BERLIN, "m", "Send invoices", &dates, "10:00");
        let o = Fx::new(BERLIN, at(BERLIN, date("2026-10-10"), "10:00"), tasks).only_offer();
        assert_eq!(o.schedule, Schedule::Monthly { time: "10:00".into(), days: vec![MonthDay::Day(3)], tz: BERLIN.into() });
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-11-03"), "10:00"));
    }

    #[test]
    fn monthly_on_the_last_day_when_short_months_end_earlier() {
        let dates = ["2026-06-30", "2026-07-31", "2026-08-31", "2026-09-30"].map(date);
        let tasks = asks(BERLIN, "m", "Close the books", &dates, "16:00");
        let o = Fx::new(BERLIN, at(BERLIN, date("2026-10-02"), "10:00"), tasks).only_offer();
        assert_eq!(o.schedule, Schedule::Monthly { time: "16:00".into(), days: vec![MonthDay::Last], tz: BERLIN.into() });
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-10-31"), "16:00"));
    }

    #[test]
    fn monthly_reads_four_months_back() {
        // 120-day window: 3 Jun is 129 days before 10 Oct and out; the rest still fit
        let dates = ["2026-06-03", "2026-07-03", "2026-08-03", "2026-09-03", "2026-10-03"].map(date);
        let d = detect(&days(&dates), date("2026-10-10")).unwrap();
        assert_eq!((d.pattern, d.month_day, d.matched.clone()), (Pattern::Monthly, Some(MonthDay::Day(3)), vec![1, 2, 3, 4]));
    }

    #[test]
    fn a_habit_on_the_31st_takes_a_30_day_months_last_day() {
        // September has no 31st: asking on the 30th is the same habit, so it runs on the last day
        let dates = ["2026-07-31", "2026-08-31", "2026-09-30"].map(date);
        let tasks = asks(BERLIN, "m", "Pay rent", &dates, "09:00");
        let o = Fx::new(BERLIN, at(BERLIN, date("2026-10-05"), "10:00"), tasks).only_offer();
        assert_eq!(o.schedule, Schedule::Monthly { time: "09:00".into(), days: vec![MonthDay::Last], tz: BERLIN.into() });
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-10-31"), "09:00"));
        // A habit on the 30th is not one on the last day
        let dates = ["2026-07-30", "2026-08-30", "2026-09-30"].map(date);
        let tasks = asks(BERLIN, "m", "Pay rent", &dates, "09:00");
        let o = Fx::new(BERLIN, at(BERLIN, date("2026-10-05"), "10:00"), tasks).only_offer();
        assert_eq!(o.schedule, Schedule::Monthly { time: "09:00".into(), days: vec![MonthDay::Day(30)], tz: BERLIN.into() });
        assert_eq!(o.first_run_at, at(BERLIN, date("2026-10-30"), "09:00"));
    }

    #[test]
    fn the_60_and_120_day_windows_end_where_they_say() {
        // Weekly: an ask 59 days back counts, one 60 days back doesn't
        let mondays = every("2026-08-24", 7, 9);
        let d = detect(&days(&mondays), date("2026-10-22")).unwrap();
        assert_eq!((d.pattern, d.matched.first()), (Pattern::Weekly, Some(&0)));
        let d = detect(&days(&mondays), date("2026-10-23")).unwrap();
        assert_eq!((d.pattern, d.matched.first(), d.matched.len()), (Pattern::Weekly, Some(&1), 8));
        // Monthly: 119 days back counts, 120 doesn't
        let thirds = ["2026-06-03", "2026-07-03", "2026-08-03", "2026-09-03"].map(date);
        let d = detect(&days(&thirds), date("2026-09-30")).unwrap();
        assert_eq!((d.pattern, d.matched.clone()), (Pattern::Monthly, vec![0, 1, 2, 3]));
        let d = detect(&days(&thirds), date("2026-10-01")).unwrap();
        assert_eq!((d.pattern, d.matched.clone()), (Pattern::Monthly, vec![1, 2, 3]));
    }

    // Near-misses at each tolerance edge

    #[test]
    fn monthly_shift_of_three_days_passes_more_does_not() {
        let today = date("2026-09-14");
        // Three days late still counts for the 3rd
        let three = ["2026-07-03", "2026-08-03", "2026-09-06"].map(date);
        assert_eq!(detect(&days(&three), today).unwrap().month_day, Some(MonthDay::Day(3)));
        // Six days apart still has a day within three of each (the 6th)
        let six = ["2026-07-03", "2026-08-03", "2026-09-09"].map(date);
        assert_eq!(detect(&days(&six), today).unwrap().month_day, Some(MonthDay::Day(6)));
        // Seven apart has none
        let seven = ["2026-07-03", "2026-08-03", "2026-09-10"].map(date);
        assert_eq!(detect(&days(&seven), today), Err(Reason::NotRegular));
    }

    #[test]
    fn weekly_shift_of_one_day_passes_two_does_not() {
        let today = date("2026-10-06");
        let mut dates = every("2026-09-07", 7, 4);
        dates[2] += Duration::days(1);
        let d = detect(&days(&dates), today).unwrap();
        assert_eq!((d.pattern, d.weekdays.clone(), d.matched.len()), (Pattern::Weekly, vec![Weekday::Mon], 4));
        dates[2] += Duration::days(1);
        assert_eq!(detect(&days(&dates), today), Err(Reason::NotRegular));
    }

    #[test]
    fn weekly_coverage_three_in_four_passes_less_does_not() {
        let today = date("2026-10-27");
        // Mondays 7 Sep to 26 Oct: 8 slots
        let all = every("2026-09-07", 7, 8);
        let six: Vec<NaiveDate> = all.iter().enumerate().filter(|(i, _)| ![2, 5].contains(i)).map(|(_, d)| *d).collect();
        let d = detect(&days(&six), today).unwrap();
        assert_eq!((d.expected, d.coverage), (8, 0.75));
        let five: Vec<NaiveDate> = all.iter().enumerate().filter(|(i, _)| ![2, 4, 5].contains(i)).map(|(_, d)| *d).collect();
        assert_eq!(detect(&days(&five), today), Err(Reason::NotRegular));
    }

    #[test]
    fn weekly_precision_one_stray_in_five_passes_more_does_not() {
        let today = date("2026-10-27");
        let mondays = every("2026-09-07", 7, 8);
        // Wednesdays, too far from a Monday to shift onto one
        let strays = every("2026-09-09", 14, 3);
        let mut two: Vec<NaiveDate> = mondays.iter().copied().chain(strays[..2].iter().copied()).collect();
        two.sort();
        let d = detect(&days(&two), today).unwrap();
        assert_eq!((d.pattern, d.weekdays.clone(), d.precision), (Pattern::Weekly, vec![Weekday::Mon], 0.8));
        let mut three: Vec<NaiveDate> = mondays.iter().copied().chain(strays.iter().copied()).collect();
        three.sort();
        // Nor are the Wednesdays a second weekday: three of eight
        assert_eq!(detect(&days(&three), today), Err(Reason::NotRegular));
    }

    #[test]
    fn a_weekday_joins_a_weekly_set_only_when_it_keeps_its_own_slots() {
        let today = date("2026-10-27");
        let mondays = every("2026-09-07", 7, 8);
        let wednesdays = every("2026-09-09", 7, 8);
        let mut both: Vec<NaiveDate> = mondays.iter().chain(&wednesdays).copied().collect();
        both.sort();
        let d = detect(&days(&both), today).unwrap();
        assert_eq!(d.weekdays, vec![Weekday::Mon, Weekday::Wed]);
        // Wednesdays on 6 of 8 weeks still hold their own slots
        let mut most: Vec<NaiveDate> = mondays.iter().chain(&wednesdays[2..]).copied().collect();
        most.sort();
        assert_eq!(detect(&days(&most), today).unwrap().weekdays, vec![Weekday::Mon, Weekday::Wed]);
    }

    #[test]
    fn weekdays_and_daily_need_their_minimums() {
        let today = date("2026-09-12");
        // Four working days: too few to tell weekdays from anything else
        let four = every("2026-09-07", 1, 4);
        assert_eq!(detect(&days(&four), today), Err(Reason::TooShort));
        // Five days in a row, all working days: weekdays, not daily
        let five = every("2026-09-07", 1, 5);
        assert_eq!(detect(&days(&five), today).unwrap().pattern, Pattern::Weekdays);
        // Seven days in a row: daily
        let seven = every("2026-09-05", 1, 7);
        assert_eq!(detect(&days(&seven), date("2026-09-12")).unwrap().pattern, Pattern::Daily);
        // Weekend asks are strays for weekdays: two workweeks and three of their four weekend
        // days are 10 of 13 (under 80%), and read as daily (13 of 14 days) instead
        let mut most = every("2026-09-07", 1, 14);
        most.retain(|d| *d != date("2026-09-13"));
        assert_eq!(detect(&days(&most), date("2026-09-21")).unwrap().pattern, Pattern::Daily);
        // Workdays with a gap each week and weekend asks: neither
        let mut gappy: Vec<NaiveDate> = every("2026-09-07", 1, 21).into_iter().filter(|d| !matches!(d.weekday(), Weekday::Wed | Weekday::Thu)).collect();
        gappy.retain(|d| *d != date("2026-09-12") && *d != date("2026-09-19"));
        assert_eq!(detect(&days(&gappy), date("2026-09-28")), Err(Reason::NotRegular));
        assert_eq!(detect(&days(&gappy[..2]), today), Err(Reason::TooShort));
    }

    // A stopped habit

    #[test]
    fn a_habit_is_stopped_once_two_of_its_slots_pass_without_an_ask() {
        // Mondays 7 Sep to 12 Oct; one missed Monday is a skipped week
        let dates = every("2026-09-07", 7, 6);
        assert_eq!(detect(&days(&dates), date("2026-10-27")).unwrap().pattern, Pattern::Weekly);
        // The 26 Oct slot is still within its one-day shift on the 27th; on the 28th it has passed
        assert_eq!(detect(&days(&dates), date("2026-10-28")), Err(Reason::Stopped));
        let tasks = asks(BERLIN, "s", "Weekly report", &dates, "09:00");
        assert_eq!(Fx::new(BERLIN, at(BERLIN, date("2026-11-02"), "09:00"), tasks).only_skip(), Reason::Stopped);
    }

    #[test]
    fn a_stopped_daily_or_weekdays_habit() {
        let daily = every("2026-09-01", 1, 14);
        assert_eq!(detect(&days(&daily), date("2026-09-16")).unwrap().pattern, Pattern::Daily);
        assert_eq!(detect(&days(&daily), date("2026-09-17")), Err(Reason::Stopped));
        // Weekdays ending Friday 2 Oct: the weekend misses nothing, Monday and Tuesday do
        let weekdays: Vec<NaiveDate> = every("2026-09-07", 1, 26).into_iter().filter(|d| d.weekday().num_days_from_monday() < 5).collect();
        assert!(detect(&days(&weekdays), date("2026-10-06")).is_ok());
        assert_eq!(detect(&days(&weekdays), date("2026-10-07")), Err(Reason::Stopped));
    }

    #[test]
    fn too_few_asks_or_none_regular() {
        let today = date("2026-10-27");
        assert_eq!(detect(&days(&every("2026-10-05", 7, 2)), today), Err(Reason::TooShort));
        let scattered = ["2026-09-01", "2026-09-04", "2026-09-15", "2026-09-26", "2026-10-09"].map(date);
        assert_eq!(detect(&days(&scattered), today), Err(Reason::NotRegular));
        // Several asks on one day are one ask
        let t = at(BERLIN, date("2026-10-05"), "09:00");
        let tasks: Vec<TaskRow> = (0..5).map(|i| task(&format!("x{i}"), "Weekly report", t + Duration::minutes(i))).collect();
        let fx = Fx::new(BERLIN, t + Duration::days(1), tasks);
        assert_eq!(fx.only_skip(), Reason::TooShort);
        assert_eq!(fx.scan().skipped[0].asks, 1);
    }

    // Gates

    fn weekly(prefix: &str, title: &str) -> Vec<TaskRow> {
        asks(BERLIN, prefix, title, &every("2026-09-07", 7, 6), "09:00")
    }

    fn after_weekly() -> DateTime<Utc> {
        at(BERLIN, date("2026-10-13"), "10:00")
    }

    #[test]
    fn needs_two_verified_and_two_thirds_of_those_with_an_outcome() {
        let mut tasks = weekly("v", "Weekly report");
        for (i, t) in tasks.iter_mut().enumerate() {
            t.state = if i < 4 { "failed" } else { "verified" }.into();
        }
        // 2 verified, 4 failed
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks.clone()).only_skip(), Reason::NotVerified);
        // 4 verified, 2 failed: exactly two thirds
        for t in tasks.iter_mut().take(2) {
            t.state = "verified".into();
        }
        let o = Fx::new(BERLIN, after_weekly(), tasks.clone()).only_offer();
        assert_eq!((o.evidence.verified, o.evidence.settled), (4, 6));
        // 1 verified, the rest still open: too few
        for (i, t) in tasks.iter_mut().enumerate() {
            t.state = if i == 5 { "verified" } else { "review" }.into();
        }
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks.clone()).only_skip(), Reason::NotVerified);
        // 2 verified, the rest open: enough
        tasks[4].state = "verified".into();
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks).only_offer().evidence.settled, 2);
    }

    #[test]
    fn one_bot_must_have_done_two_thirds_of_the_verified_ones() {
        let mut tasks = weekly("o", "Weekly report");
        for t in tasks.iter_mut().take(2) {
            t.owner_kwant_id = Some("bot-b".into());
        }
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks.clone()).only_offer().kwant_id, "bot-a");
        tasks[2].owner_kwant_id = Some("bot-b".into());
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks.clone()).only_skip(), Reason::MixedOwners);
        for t in &mut tasks {
            t.owner_kwant_id = None;
        }
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks).only_skip(), Reason::MixedOwners);
    }

    #[test]
    fn a_bot_no_longer_in_the_space_gets_no_offer() {
        let mut fx = Fx::new(BERLIN, after_weekly(), weekly("u", "Weekly report"));
        fx.bots.remove("bot-a");
        assert_eq!(fx.only_skip(), Reason::BotUnavailable);
    }

    fn routine(kwant: &str, name: &str) -> RoutineRef {
        RoutineRef { kwant_id: kwant.into(), name: name.into(), instructions: String::new(), task_class: None, cluster_key: None }
    }

    #[test]
    fn covered_by_a_routine_on_or_off_of_the_same_habit() {
        let tasks = weekly("c", "Weekly sales report");
        let key = cluster_key("general", "Weekly sales report").unwrap();
        let mut fx = Fx::new(BERLIN, after_weekly(), tasks);
        // Accepted from this habit, whatever its bot or name
        fx.routines = vec![RoutineRef { cluster_key: Some(key), ..routine("bot-b", "Something else") }];
        assert_eq!(fx.only_skip(), Reason::Covered);
        // Named for it, same bot
        fx.routines = vec![routine("bot-a", "Sales report weekly")];
        assert_eq!(fx.only_skip(), Reason::Covered);
        // Its instructions start with it: 3 of 5 words is the 0.6 edge
        fx.routines = vec![RoutineRef { instructions: "Weekly sales report, as usual\nUse the CRM.".into(), ..routine("bot-a", "Mondays") }];
        assert_eq!(fx.only_skip(), Reason::Covered);
        // Another bot's routine of that name, or an unrelated one, doesn't cover it
        fx.routines = vec![routine("bot-b", "Weekly sales report"), routine("bot-a", "Weekly hiring report")];
        assert_eq!(fx.only_offer().cluster_key, cluster_key("general", "Weekly sales report").unwrap());
    }

    #[test]
    fn covered_by_class() {
        let mut tasks = weekly("k", "Invoices");
        for t in &mut tasks {
            t.task_class = "invoice".into();
        }
        let mut fx = Fx::new(BERLIN, after_weekly(), tasks);
        let o = fx.only_offer();
        assert_eq!((o.cluster_key.as_str(), o.task_class.as_str()), ("class:invoice", "invoice"));
        fx.routines = vec![RoutineRef { task_class: Some("invoice".into()), ..routine("bot-a", "Billing") }];
        assert_eq!(fx.only_skip(), Reason::Covered);
    }

    #[test]
    fn cooling_off_and_already_offered() {
        let key = cluster_key("general", "Weekly report").unwrap();
        let mut fx = Fx::new(BERLIN, after_weekly(), weekly("d", "Weekly report"));
        fx.past.cooling.insert(key.clone());
        assert_eq!(fx.only_skip(), Reason::CoolingOff);
        fx.past = Past { live: HashSet::from([key.clone()]), live_count: 1, ..Past::default() };
        assert_eq!(fx.only_skip(), Reason::AlreadyOffered);
        // Still passing, so the caller keeps the live offer; it is not offered again
        let scan = fx.scan();
        assert_eq!(scan.live.len(), 1);
        assert_eq!(scan.live[0].cluster_key, key);
        assert_eq!(scan.live[0].evidence.last_ask_at, at(BERLIN, date("2026-10-12"), "09:00"));
        // Cooling wins over live, so a live offer of a cooled habit isn't in `live` (it expires)
        fx.past.cooling.insert(key);
        assert!(fx.scan().live.is_empty());
    }

    #[test]
    fn a_live_offer_is_replaced_only_for_a_real_change() {
        let fx = Fx::new(BERLIN, after_weekly(), weekly("m", "Weekly report"));
        let now = fx.only_offer();
        let weekly_at = |time: &str, days: &[&str], tz: &str| Schedule::Weekly { time: time.into(), weekdays: days.iter().map(|d| d.to_string()).collect(), tz: tz.into() };
        assert!(!materially_changed(&now.schedule, "bot-a", &now));
        // The median drifting under half an hour keeps the offer
        assert!(!materially_changed(&weekly_at("09:15", &["mon"], BERLIN), "bot-a", &now));
        assert!(!materially_changed(&weekly_at("08:31", &["mon"], BERLIN), "bot-a", &now));
        assert!(materially_changed(&weekly_at("09:30", &["mon"], BERLIN), "bot-a", &now));
        assert!(materially_changed(&weekly_at("09:00", &["mon", "thu"], BERLIN), "bot-a", &now));
        assert!(materially_changed(&weekly_at("09:00", &["mon"], "America/New_York"), "bot-a", &now));
        assert!(materially_changed(&Schedule::Daily { time: "09:00".into(), tz: BERLIN.into() }, "bot-a", &now));
        assert!(materially_changed(&now.schedule, "bot-b", &now));
    }

    #[test]
    fn the_cap_keeps_the_strongest_habits() {
        let mut tasks = weekly("a", "Weekly sales report");
        tasks.extend(asks(BERLIN, "b", "Check server backups", &every("2026-08-25", 7, 8), "11:00"));
        tasks.extend(asks(BERLIN, "c", "Update hiring pipeline", &every("2026-09-09", 7, 5), "15:00"));
        let mut fx = Fx::new(BERLIN, after_weekly(), tasks);
        let names = |s: &Scan| s.offered.iter().map(|o| o.name.clone()).collect::<Vec<_>>();
        fx.cap = 3;
        assert_eq!(names(&fx.scan()), ["Check server backups", "Weekly sales report", "Update hiring pipeline"]);
        // Two a day
        fx.cap = 2;
        let scan = fx.scan();
        assert_eq!(names(&scan), ["Check server backups", "Weekly sales report"]);
        assert_eq!(scan.skipped.iter().map(|s| (s.title.as_str(), s.reason)).collect::<Vec<_>>(), [("Update hiring pipeline", Reason::Capped)]);
        // One made today already
        fx.past.offered_24h = 1;
        assert_eq!(names(&fx.scan()), ["Check server backups"]);
        // Five live in the space
        fx.past = Past { live_count: 5, ..Past::default() };
        let scan = fx.scan();
        assert!(scan.offered.is_empty());
        assert!(scan.skipped.iter().all(|s| s.reason == Reason::Capped));
        // Cap 0: never
        fx.past = Past::default();
        fx.cap = 0;
        assert!(fx.scan().offered.is_empty());
    }

    // Chains

    fn with_follow_up(owner: &str) -> Vec<TaskRow> {
        let mut tasks = weekly("a", "Weekly sales report");
        let follow: Vec<TaskRow> = tasks
            .iter()
            .enumerate()
            .map(|(i, a)| TaskRow { owner_kwant_id: Some(owner.into()), ..task(&format!("b-{i}"), "Post the sales summary to the team", a.finished_at.unwrap() + Duration::hours(2)) })
            .collect();
        tasks.extend(follow);
        tasks
    }

    #[test]
    fn a_follow_up_by_the_same_bot_becomes_a_step() {
        let scan = Fx::new(BERLIN, after_weekly(), with_follow_up("bot-a")).scan();
        assert_eq!(scan.offered.len(), 1);
        let o = &scan.offered[0];
        assert_eq!(o.name, "Weekly sales report");
        assert_eq!(o.chain.len(), 1);
        assert!(o.chain[0].in_instructions);
        assert_eq!(o.chain[0].after, 6);
        assert!(o.instructions.contains("Then, in the same run:\n1. Post the sales summary to the team"), "{}", o.instructions);
        // The follow-up isn't offered on its own as well
        let b = cluster_key("general", "Post the sales summary to the team").unwrap();
        assert_eq!(scan.skipped.iter().map(|s| (s.cluster_key.clone(), s.reason)).collect::<Vec<_>>(), [(b, Reason::InChain)]);
    }

    #[test]
    fn a_follow_up_by_another_bot_is_mentioned_not_a_step() {
        let scan = Fx::new(BERLIN, after_weekly(), with_follow_up("bot-b")).scan();
        let a = scan.offered.iter().find(|o| o.name == "Weekly sales report").unwrap();
        assert_eq!(a.chain.len(), 1);
        assert_eq!(a.chain[0].kwant_id.as_deref(), Some("bot-b"));
        assert!(!a.chain[0].in_instructions);
        assert!(!a.instructions.contains("Post the sales summary"), "{}", a.instructions);
        assert!(!a.instructions.contains("Then, in the same run"));
        // It is its own habit, of its own bot
        let b = scan.offered.iter().find(|o| o.name == "Post the sales summary to the team").unwrap();
        assert_eq!(b.kwant_id, "bot-b");
    }

    #[test]
    fn a_follow_up_needs_to_come_after_most_asks_within_a_day() {
        // After 4 of 6 asks: under 80%
        let mut tasks = with_follow_up("bot-a");
        tasks.retain(|t| t.id != "b-0" && t.id != "b-3");
        let scan = Fx::new(BERLIN, after_weekly(), tasks).scan();
        assert!(scan.offered.iter().find(|o| o.name == "Weekly sales report").unwrap().chain.is_empty());
        // More than a day after the ask was done
        let mut late = with_follow_up("bot-a");
        for t in late.iter_mut().filter(|t| t.id.starts_with("b-")) {
            t.created_at += Duration::hours(23);
        }
        let scan = Fx::new(BERLIN, after_weekly(), late).scan();
        assert!(scan.offered.iter().all(|o| o.chain.is_empty()));
    }

    #[test]
    fn a_live_offers_step_is_not_offered_again() {
        let mut fx = Fx::new(BERLIN, after_weekly(), weekly("b", "Post the sales summary to the team"));
        fx.past.live_chain.insert(cluster_key("general", "Post the sales summary to the team").unwrap());
        assert_eq!(fx.only_skip(), Reason::InChain);
    }

    // What the offer carries

    #[test]
    fn checks_that_passed_with_the_same_spec_twice_are_kept() {
        let tasks = weekly("t", "Weekly report");
        let mut checks = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            let c = |kind: &str, spec: String| PassedCheck { task_id: t.id.clone(), kind: kind.into(), spec, required: true };
            checks.push(c("file_exists", r#"{"path":"reports/weekly.md"}"#.into()));
            checks.push(c("person", r#"{"question":"Looks right?"}"#.into()));
            checks.push(c("file_contains", format!(r#"{{"path":"reports/{i}.md","text":"Total"}}"#)));
            if i == 0 {
                checks.push(c("http", r#"{"url":"https://example.com/health"}"#.into()));
            }
        }
        let mut fx = Fx::new(BERLIN, after_weekly(), tasks);
        fx.checks = checks;
        let o = fx.only_offer();
        assert_eq!(o.checks, vec![CheckSpec { kind: "file_exists".into(), spec: serde_json::json!({"path": "reports/weekly.md"}), required: true }]);
        let left: Vec<(&str, &str)> = o.left_out_checks.iter().map(|l| (l.kind.as_str(), l.reason)).collect();
        assert_eq!(left, [("person", "needs a person or a reviewer"), ("file_contains", "changes every time"), ("http", "passed only once")]);
        assert_eq!(o.acceptance, "The report is in the shared folder");
        assert!(o.instructions.starts_with("Weekly report\n\nA good result: The report is in the shared folder"));
        assert!(o.instructions.contains("Based on your tasks: \"Weekly report 2026-10-12\" (2026-10-12)"), "{}", o.instructions);
    }

    #[test]
    fn delivery_follows_where_the_asks_were_made() {
        let mut tasks = weekly("p", "Weekly report");
        let fx = Fx::new(BERLIN, after_weekly(), tasks.clone());
        assert_eq!(fx.only_offer().delivery, Delivery { direct_kwant_id: Some("bot-a".into()), ..Default::default() });
        for t in &mut tasks {
            t.channel_id = Some("room-1".into());
        }
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks.clone()).only_offer().delivery, Delivery { channel_id: Some("room-1".into()), ..Default::default() });
        for t in tasks.iter_mut().skip(2) {
            t.thread_id = Some("thread-9".into());
        }
        assert_eq!(Fx::new(BERLIN, after_weekly(), tasks).only_offer().delivery, Delivery { thread_id: Some("thread-9".into()), ..Default::default() });
    }

    #[test]
    fn times_far_apart_are_marked_as_varying() {
        let mut tasks = weekly("t", "Weekly report");
        tasks[1].created_at += Duration::hours(5);
        let o = Fx::new(BERLIN, after_weekly(), tasks).only_offer();
        assert_eq!(o.evidence.time_spread, "09:00–14:00");
        assert!(o.evidence.time_varies);
        assert_eq!(o.schedule.tz(), Some(BERLIN));
    }

    #[test]
    fn first_run_skips_this_periods_ask_already_done() {
        let s = Schedule::Daily { time: "07:00".into(), tz: BERLIN.into() };
        let asked = at(BERLIN, date("2026-10-02"), "07:10");
        // Asked at 07:10, now 06:00 the next day: the next run is today 07:00
        assert_eq!(first_run_at(&s, asked, at(BERLIN, date("2026-10-03"), "06:00")), Some(at(BERLIN, date("2026-10-03"), "07:00")));
        // Asked at 06:00 before the usual time: not again at 07:00 the same day
        let early = at(BERLIN, date("2026-10-02"), "06:00");
        assert_eq!(first_run_at(&s, early, early + Duration::minutes(5)), Some(at(BERLIN, date("2026-10-03"), "07:00")));
        let w = Schedule::Weekly { time: "09:00".into(), weekdays: vec!["mon".into()], tz: BERLIN.into() };
        let monday = at(BERLIN, date("2026-10-05"), "08:00");
        assert_eq!(first_run_at(&w, monday, monday + Duration::hours(1)), Some(at(BERLIN, date("2026-10-12"), "09:00")));
    }

    #[test]
    fn first_run_is_recomputed_when_accepted_days_later() {
        // Offered on Monday 10:00 after that Monday's 09:00 ask: the stored first run is next Monday
        let w = Schedule::Weekly { time: "09:00".into(), weekdays: vec!["mon".into()], tz: BERLIN.into() };
        let asked = at(BERLIN, date("2026-10-12"), "09:00");
        let stored = first_run_at(&w, asked, at(BERLIN, date("2026-10-12"), "10:00")).unwrap();
        assert_eq!(stored, at(BERLIN, date("2026-10-19"), "09:00"));
        // Accepted the Wednesday after that Monday: the following Monday, not the stale one
        let wednesday = at(BERLIN, date("2026-10-21"), "11:00");
        assert_eq!(first_run_at(&w, asked, wednesday), Some(at(BERLIN, date("2026-10-26"), "09:00")));
        // Edited to Thursdays: the next Thursday
        let thu = Schedule::Weekly { time: "09:00".into(), weekdays: vec!["thu".into()], tz: BERLIN.into() };
        assert_eq!(first_run_at(&thu, asked, wednesday), Some(at(BERLIN, date("2026-10-22"), "09:00")));
        // Across the 25 October DST change it stays 09:00 local
        assert_eq!(first_run_at(&w, asked, at(BERLIN, date("2026-10-20"), "08:00")), Some(at(BERLIN, date("2026-10-26"), "09:00")));
    }

    #[test]
    fn an_unreadable_time_zone_finds_nothing() {
        let fx = Fx { tz: "Mars/Olympus", ..Fx::new(BERLIN, after_weekly(), weekly("z", "Weekly report")) };
        assert_eq!(fx.scan(), Scan::default());
    }

    #[test]
    fn the_same_rows_give_the_same_scan() {
        let mut tasks = with_follow_up("bot-b");
        tasks.extend(asks(BERLIN, "c", "Update hiring pipeline", &every("2026-09-09", 7, 5), "15:00"));
        let fx = Fx::new(BERLIN, after_weekly(), tasks.clone());
        tasks.reverse();
        let reversed = Fx::new(BERLIN, after_weekly(), tasks);
        assert_eq!(fx.scan(), reversed.scan());
    }
}
