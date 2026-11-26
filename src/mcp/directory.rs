//! The public MCP directory as people search it ("Find a tool", mcp-finder): a local copy of the
//! official MCP Registry's latest versions, synced in the background, and the matching and
//! ranking that turn a few typed letters into up to 12 rows in well under 50 ms.
//!
//! The copy is made on demand: startup only arms the sync ([`Directory::arm`]) and the first
//! search starts it, so a kwnt where nobody opens Find a tool never holds the registry in memory
//! (perf-idle-memory: holding it from startup cost ~35 MB of idle memory on CI, and the whole
//! registry is ~75,000 entries). The first sync pages through the whole registry; after that
//! kwnt asks only for what changed (`updated_since`) every 6 hours. Until the first sync has
//! finished, a search goes to the registry's own (name-only) search instead. Only entries kwnt
//! can set up in one click are kept.
//!
//! Ranking: entries from Smithery's proxy or with a `{placeholder}` address go last; then a
//! publisher whose proven name is the query (`stripe` → stripe.com, `github` → github.com/github);
//! then how well the name matches (starts with > a word starts with > alias > a part of the
//! registry name > contains > typo); then a publisher a typo away (`strpie` → stripe.com), which
//! only orders entries within one of those tiers; then an exact name or title; then remote before
//! stdio; then registry order. Typos are only looked for when closer matches leave room.

use super::registry::{publisher_name, McpRegistry, RegistryEntry, GENERIC_SERVER_PARTS};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};
use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

/// Most entries one search answers.
pub const MAX_RESULTS: usize = 12;
/// Shorter queries answer nothing (the UI asks from 2 letters).
pub const MIN_QUERY_CHARS: usize = 2;
const SYNC_EVERY: Duration = Duration::from_secs(6 * 3600);
/// After a failed sync: first retry, doubling up to [`RETRY_MAX`].
const RETRY_FIRST: Duration = Duration::from_secs(120);
const RETRY_MAX: Duration = Duration::from_secs(3600);
/// A pause between pages, to be gentle with a free service.
const PAGE_PAUSE: Duration = Duration::from_millis(150);
/// Safety stops for a registry that never ends. Not a cap on the real one: it had 749 pages and
/// 74,866 entries on Nov 26, 2026 (the old 60,000 stop cut off the end of the alphabet), and a
/// kept entry is now ~200 bytes in memory (perf-idle-memory A2).
const MAX_PAGES: usize = 5000;
const MAX_ENTRIES: usize = 400_000;
/// `updated_since` starts this much before the last sync began, so nothing falls in a gap.
const OVERLAP: Duration = Duration::from_secs(600);

/// The publisher line: what the registry's namespace check proves about who published it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Publisher {
    /// "stripe.com" or "github.com/alice".
    pub label: String,
    /// "domain" or "github".
    pub kind: &'static str,
}

/// `com.stripe/mcp` → stripe.com; `io.github.alice/x` → github.com/alice.
pub fn publisher_of(registry_name: &str) -> Publisher {
    let ns = registry_name.split('/').next().unwrap_or(registry_name).to_ascii_lowercase();
    match ns.strip_prefix("io.github.") {
        Some(user) => Publisher { label: format!("github.com/{user}"), kind: "github" },
        None => Publisher { label: ns.rsplit('.').collect::<Vec<_>>().join("."), kind: "domain" },
    }
}

/// Lower case, no accents, letters and digits only: "Google Sheets" = "google-sheets" = "googlesheets".
pub fn normalize(text: &str) -> String {
    text.nfd().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).map(normalize).filter(|w| !w.is_empty()).collect()
}

/// Other names people type (autocomplete.md §4); the value is matched like a typed query.
const ALIASES: [(&str, &str); 13] = [
    ("googlemail", "gmail"),
    ("gcal", "google calendar"),
    ("calendar", "google calendar"),
    ("sheets", "google sheets"),
    ("docs", "google docs"),
    ("drive", "google drive"),
    ("11labs", "elevenlabs"),
    ("eleven", "elevenlabs"),
    ("teams", "microsoft teams"),
    ("twitter", "x"),
    ("postgres", "postgresql"),
    ("pg", "postgresql"),
    ("gdrive", "google drive"),
];

/// How well a name matches; lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    Prefix,
    WordPrefix,
    Alias,
    NamePart,
    Contains,
    /// A whole word, the title or the name is a typo away.
    Typo,
    /// Only the start of a word is a typo away.
    TypoStart,
}

/// What matching needs about one entry, packed into one string (perf-idle-memory A2): the
/// registry name, then the title, the registry name, the words (space separated) and the
/// publisher label, all normalised. The slug and the publisher key aren't stored: they are the
/// end of the normalised name and the start or end of the normalised label. About 160 bytes an
/// entry, where a whole [`RegistryEntry`] with its plan and seven separate strings took ~2.5 KB
/// (a full copy of the registry was +153 MB). The rows to show live in [`RowFile`] on disk until
/// they are a hit.
///
/// A copy's keys all share one string ([`pack`]); an entry on its own (the live search) has its
/// own. One allocation for 75,000 keys instead of 75,000 also lets the memory a sync used go
/// back to the system (A2 round 3: CI's after-search memory was 160 MB against a 150 limit).
#[derive(Debug, Clone)]
pub struct Key {
    text: Arc<String>,
    /// Where this key starts in `text`.
    start: u32,
    /// Where each part ends, from `start`: name, title, name normalised, words, publisher label
    /// (the end of the key).
    ends: [u16; 5],
    /// The slug normalised is the last `slug` bytes of the name normalised.
    slug: u16,
    /// The publisher key is the first (a domain) or last (GitHub) `publisher_key` bytes of the
    /// publisher label normalised; 0 for none.
    publisher_key: u16,
    publisher_key_at_end: bool,
    demoted: bool,
    remote: bool,
}

impl Key {
    /// `None` when the text would not fit the packed offsets (64 KB: no real entry is near it).
    fn new(name: &str, title: &str, publisher: &Publisher, demoted: bool, remote: bool) -> Option<Self> {
        let slug = name.rsplit('/').next().unwrap_or(name);
        let mut ws = words(title);
        for w in words(slug) {
            if !ws.contains(&w) {
                ws.push(w);
            }
        }
        let name_norm = normalize(name);
        let publisher_norm = normalize(&publisher.label);
        // normalize works char by char and '/' and '.' are never moved past, so the slug is the
        // end of the name and the publisher key the start (stripe of stripe.com) or end (alice of
        // github.com/alice) of the label. Checked anyway: such an entry is left out, never misread.
        let slug_norm = normalize(slug);
        if !name_norm.ends_with(&slug_norm) {
            return None;
        }
        let pk = publisher_key(publisher).unwrap_or_default();
        let at_end = publisher.kind == "github";
        if !pk.is_empty() && !(if at_end { publisher_norm.ends_with(&pk) } else { publisher_norm.starts_with(&pk) }) {
            return None;
        }
        let parts = [name.to_string(), normalize(title), name_norm, ws.join(" "), publisher_norm];
        let mut text = String::with_capacity(parts.iter().map(String::len).sum());
        let mut ends = [0u16; 5];
        for (end, part) in ends.iter_mut().zip(&parts) {
            text.push_str(part);
            *end = u16::try_from(text.len()).ok()?;
        }
        Some(Self {
            text: Arc::new(text),
            start: 0,
            ends,
            slug: u16::try_from(slug_norm.len()).ok()?,
            publisher_key: u16::try_from(pk.len()).ok()?,
            publisher_key_at_end: at_end,
            demoted,
            remote,
        })
    }

    /// The whole key, every part one after another.
    fn all(&self) -> &str {
        let start = self.start as usize;
        &self.text[start..start + usize::from(self.ends[4])]
    }

    fn part(&self, i: usize) -> &str {
        let from = if i == 0 { 0 } else { usize::from(self.ends[i - 1]) };
        &self.all()[from..usize::from(self.ends[i])]
    }

    /// The registry name, as the registry spells it.
    pub fn name(&self) -> &str {
        self.part(0)
    }
    fn title_norm(&self) -> &str {
        self.part(1)
    }
    fn name_norm(&self) -> &str {
        self.part(2)
    }
    fn slug_norm(&self) -> &str {
        let n = self.name_norm();
        &n[n.len() - usize::from(self.slug)..]
    }
    fn words(&self) -> impl Iterator<Item = &str> + Clone {
        self.part(3).split(' ').filter(|w| !w.is_empty())
    }
    /// The publisher label normalised (`stripecom`).
    fn publisher_norm(&self) -> &str {
        self.part(4)
    }
    /// The publisher's own name: `stripe` for stripe.com (two-label domains only, so a subdomain
    /// such as pg.ddx.io or stripecheckup.vercel.app isn't taken for its owner), `getsentry` for
    /// github.com/getsentry.
    fn publisher_key(&self) -> Option<&str> {
        let (p, k) = (self.publisher_norm(), usize::from(self.publisher_key));
        match k {
            0 => None,
            _ if self.publisher_key_at_end => Some(&p[p.len() - k..]),
            _ => Some(&p[..k]),
        }
    }
    /// Dot and slash separated parts of the registry name, normalised. Worked out when asked:
    /// only an entry that already contains the query gets this far.
    fn parts(&self) -> impl Iterator<Item = String> + '_ {
        self.name().split(['.', '/']).map(normalize).filter(|p| !p.is_empty())
    }

    /// The publisher is the query itself (`stripe` → stripe.com).
    fn publisher_is(&self, q: &str) -> bool {
        self.publisher_key() == Some(q) || self.publisher_norm() == q
    }

    fn direct_match(&self, q: &str) -> Option<Match> {
        if self.title_norm().starts_with(q) || self.slug_norm().starts_with(q) || self.publisher_is(q) {
            Some(Match::Prefix)
        } else if self.words().any(|w| w.starts_with(q)) {
            Some(Match::WordPrefix)
        } else {
            None
        }
    }

    /// Every match but a typo.
    fn close_match(&self, q: &Query) -> Option<Match> {
        // The title, its words, the registry name, its parts and the publisher's name are all
        // pieces of the normalised title or name; only the publisher label (`stripecom`) isn't.
        let may = |t: &str| self.title_norm().contains(t) || self.name_norm().contains(t) || self.publisher_norm() == t;
        if !may(&q.text) && !q.alias.as_deref().is_some_and(may) {
            return None;
        }
        if let Some(m) = self.direct_match(&q.text) {
            return Some(m);
        }
        if q.alias.as_deref().is_some_and(|a| self.direct_match(a).is_some()) {
            return Some(Match::Alias);
        }
        if self.parts().any(|p| p.starts_with(q.text.as_str())) {
            return Some(Match::NamePart);
        }
        if self.title_norm().contains(q.text.as_str()) || self.name_norm().contains(q.text.as_str()) {
            return Some(Match::Contains);
        }
        None
    }

    fn typo_match(&self, q: &Query) -> Option<Match> {
        if q.budget == 0 {
            return None;
        }
        let all = || self.words().chain([self.title_norm(), self.slug_norm()]);
        let whole = |w: &str| within(&q.text, q.chars, w, q.budget);
        if all().any(whole) {
            return Some(Match::Typo);
        }
        let start = |w: &str| match w.char_indices().nth(q.chars) {
            Some((end, _)) if q.budget == 1 => one_edit_same_length(&q.text, &w[..end]),
            Some((end, _)) => osa_within(&q.text, &w[..end], q.budget),
            None => false,
        };
        all().any(start).then_some(Match::TypoStart)
    }

    /// How `q` matches this entry, if at all.
    pub fn matches(&self, q: &str) -> Option<Match> {
        let q = Query::new(q)?;
        self.close_match(&q).or_else(|| self.typo_match(&q))
    }

    fn exact(&self, q: &str) -> bool {
        self.title_norm() == q || self.slug_norm() == q
    }

    /// The sort key; lower comes first.
    fn key(&self, q: &Query, m: Match, order: usize) -> SortKey {
        let publisher = self.publisher_is(&q.text);
        let publisher_typo = !publisher && q.budget > 0 && self.publisher_key().is_some_and(|k| within(&q.text, q.chars, k, q.budget));
        // Among typo matches, the name or title itself a typo away (`gmial` → gmail) first
        let exact = self.exact(&q.text) || (m == Match::Typo && [self.title_norm(), self.slug_norm()].iter().any(|t| within(&q.text, q.chars, t, q.budget)));
        (self.demoted, !publisher, m, !publisher_typo, !exact, !self.remote, order)
    }
}

impl AsRef<Key> for Key {
    fn as_ref(&self) -> &Key {
        self
    }
}

/// One input the person fills in on the form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RowInput {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub secret: bool,
}

/// What Find a tool shows for one entry (and what "already added" compares). Installing fetches
/// the entry from the registry again, so nothing else is kept.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub registry_name: String,
    pub version: String,
    /// The registry's title, else a name made from its registry name.
    pub title: String,
    pub description: String,
    /// "http" or "stdio".
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub inputs: Vec<RowInput>,
}

impl Row {
    pub fn publisher(&self) -> Publisher {
        publisher_of(&self.registry_name)
    }
    pub fn is_remote(&self) -> bool {
        self.transport == "http"
    }
}

/// One entry as a search ranks it: what matching needs and the row to show.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub row: Row,
    pub publisher: Publisher,
    key: Key,
}

impl AsRef<Key> for DirEntry {
    fn as_ref(&self) -> &Key {
        &self.key
    }
}

/// The title to show: the registry's own, else the part after the slash, else for a generic
/// part (`com.stripe/mcp`) the publisher's name, capitalised ("Stripe").
fn display_title(e: &RegistryEntry) -> String {
    if e.title != e.name && !e.title.trim().is_empty() {
        return e.title.clone();
    }
    let part = e.name.rsplit('/').next().unwrap_or(&e.name);
    if !GENERIC_SERVER_PARTS.contains(&part.to_ascii_lowercase().as_str()) {
        return part.to_string();
    }
    let p = publisher_name(&e.name);
    let mut c = p.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_else(|| e.name.clone())
}

impl DirEntry {
    /// `None` for an entry kwnt can't set up in one click: the directory doesn't show those.
    pub fn new(entry: RegistryEntry) -> Option<Self> {
        let plan = entry.plan.as_ref().ok()?;
        let publisher = publisher_of(&entry.name);
        let title = display_title(&entry);
        let demoted = entry.name.to_ascii_lowercase().starts_with("ai.smithery/") || entry.placeholder_url;
        let key = Key::new(&entry.name, &title, &publisher, demoted, plan.transport == "http")?;
        let row = Row {
            transport: plan.transport.clone(),
            url: plan.url.clone(),
            command: plan.command.clone(),
            args: plan.args.clone(),
            inputs: plan
                .inputs
                .iter()
                .map(|i| RowInput { name: i.name.clone(), description: i.description.clone(), required: i.required, secret: i.secret })
                .collect(),
            registry_name: entry.name,
            version: entry.version,
            title,
            description: entry.description,
        };
        Some(Self { row, publisher, key })
    }

    /// How `q` matches this entry, if at all.
    pub fn matches(&self, q: &str) -> Option<Match> {
        self.key.matches(q)
    }
}

/// `stripe` for stripe.com, `getsentry` for github.com/getsentry; none for a longer domain.
fn publisher_key(p: &Publisher) -> Option<String> {
    let key = match p.kind {
        "github" => normalize(p.label.strip_prefix("github.com/")?),
        _ => match p.label.split('.').collect::<Vec<_>>()[..] {
            [name, _tld] => normalize(name),
            _ => return None,
        },
    };
    (!key.is_empty()).then_some(key)
}

/// Demoted, not the publisher, match, no publisher a typo away, not exact, not remote, order.
type SortKey = (bool, bool, Match, bool, bool, bool, usize);

/// A typed query, worked out once per search.
struct Query {
    /// Normalised.
    text: String,
    chars: usize,
    /// What an alias stands for, normalised (`pg` → `postgresql`).
    alias: Option<String>,
    budget: usize,
}

impl Query {
    /// `None` below [`MIN_QUERY_CHARS`].
    fn new(typed: &str) -> Option<Self> {
        let text = normalize(typed);
        let chars = text.chars().count();
        if chars < MIN_QUERY_CHARS {
            return None;
        }
        let alias = ALIASES.iter().find(|(a, _)| *a == text).map(|(_, canonical)| normalize(canonical));
        Some(Self { budget: typo_budget(chars), text, chars, alias })
    }
}

/// 1 edit from 4 letters, 2 from 8 (autocomplete.md §4); none past [`OSA_MAX`] letters.
fn typo_budget(chars: usize) -> usize {
    match chars {
        0..=3 => 0,
        4..=7 => 1,
        8..=OSA_MAX => 2,
        _ => 0,
    }
}

fn char_count(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// Two strings of the same length one substitution or one swap of neighbours apart (or equal):
/// [`osa_bounded`] ≤ 1 for them, without the table.
fn one_edit_same_length(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (a.chars(), b.chars());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => {
                if a.clone().eq(b.clone()) {
                    return true;
                }
                return matches!((a.next(), b.next()), (Some(x2), Some(y2)) if x2 == y && y2 == x) && a.eq(b);
            }
            _ => return false,
        }
    }
}

/// `w` at most `max` edits from `q` (`q_chars` characters long). One edit, the common case, is
/// checked without the table.
fn within(q: &str, q_chars: usize, w: &str, max: usize) -> bool {
    let w_chars = char_count(w);
    if w_chars.abs_diff(q_chars) > max {
        return false;
    }
    match (max, w_chars.cmp(&q_chars)) {
        (0, _) => q == w,
        (1, std::cmp::Ordering::Equal) => one_edit_same_length(q, w),
        (1, std::cmp::Ordering::Less) => one_insert(w, q),
        (1, std::cmp::Ordering::Greater) => one_insert(q, w),
        _ => osa_within(q, w, max),
    }
}

/// `long` is `short` with one character added.
fn one_insert(short: &str, long: &str) -> bool {
    let (mut s, mut l) = (short.chars(), long.chars());
    loop {
        match (s.clone().next(), l.next()) {
            (Some(x), Some(y)) if x == y => {
                s.next();
            }
            (_, Some(_)) => return s.eq(l),
            (_, None) => return false,
        }
    }
}

/// The longest strings [`osa_bounded`] compares.
const OSA_MAX: usize = 40;

fn osa_within(a: &str, b: &str, max: usize) -> bool {
    osa_bounded(a, b, max) <= max
}

/// Edits (insert, delete, substitute, swap two neighbours) between two short strings, without
/// allocating; anything over `max`, or a string longer than [`OSA_MAX`], answers `max + 1`.
fn osa_bounded(a: &str, b: &str, max: usize) -> usize {
    fn fill(s: &str, buf: &mut [char; OSA_MAX]) -> Option<usize> {
        let mut n = 0;
        for c in s.chars() {
            *buf.get_mut(n)? = c;
            n += 1;
        }
        Some(n)
    }
    let (mut ab, mut bb) = (['\0'; OSA_MAX], ['\0'; OSA_MAX]);
    let (Some(n), Some(m)) = (fill(a, &mut ab), fill(b, &mut bb)) else { return max + 1 };
    if n.abs_diff(m) > max {
        return max + 1;
    }
    let (a, b) = (&ab[..n], &bb[..m]);
    let (mut before, mut prev, mut cur) = ([0usize; OSA_MAX + 1], [0usize; OSA_MAX + 1], [0usize; OSA_MAX + 1]);
    for (j, cell) in prev.iter_mut().enumerate().take(m + 1) {
        *cell = j;
    }
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(before[j - 2] + 1);
            }
            cur[j] = d;
            row_min = row_min.min(d);
        }
        // A row's minimum never drops by more than one per row, so this can't come back under.
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut before, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m].min(max + 1)
}

/// The best `limit` entries for `query`, in order. `entries` is in registry order.
pub fn rank<'a, T: AsRef<Key>>(query: &str, entries: &'a [T], limit: usize) -> Vec<&'a T> {
    let Some(q) = Query::new(query) else { return Vec::new() };
    let mut hits = Vec::new();
    // Only the place of each entry that didn't match closely (4 bytes, not a 16-byte reference:
    // a search of a 75,000-entry copy would hold 1.2 MB for it)
    let mut rest: Vec<u32> = Vec::new();
    let mut shown = 0usize;
    for (order, e) in entries.iter().enumerate() {
        let k = e.as_ref();
        match k.close_match(&q) {
            Some(m) => {
                shown += usize::from(!k.demoted);
                hits.push((k.key(&q, m, order), e));
            }
            None if q.budget > 0 => rest.extend(u32::try_from(order).ok()),
            None => {}
        }
    }
    // A typo match ranks below every closer match that isn't demoted, so typos are only
    // looked for when those leave room.
    if shown < limit {
        hits.extend(rest.into_iter().filter_map(|order| {
            let (order, e) = (order as usize, &entries[order as usize]);
            e.as_ref().typo_match(&q).map(|m| (e.as_ref().key(&q, m, order), e))
        }));
    }
    hits.sort_unstable_by_key(|h| h.0);
    hits.into_iter().take(limit).map(|(_, e)| e).collect()
}

/// Where one entry's [`Row`] sits in its [`RowFile`].
#[derive(Debug, Clone, Copy)]
struct RowAt {
    offset: u32,
    len: u32,
}

/// One entry of the copy: its packed [`Key`] in memory, its row on disk.
#[derive(Debug, Clone)]
pub struct Item {
    key: Key,
    row: RowAt,
}

impl AsRef<Key> for Item {
    fn as_ref(&self) -> &Key {
        &self.key
    }
}

/// The rows of a copy, one JSON object after another in a file of kwnt's own (perf-idle-memory
/// A2: in memory they were most of the +153 MB a full copy cost). A search reads back only its
/// hits. Written once by the first sync, then only appended to; removed when the last copy
/// that uses it goes.
pub struct RowFile {
    path: PathBuf,
    file: Mutex<File>,
}

impl std::fmt::Debug for RowFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowFile").field("path", &self.path).finish()
    }
}

/// The largest a row file may grow (offsets are 32 bits).
const MAX_ROW_FILE: u64 = u32::MAX as u64;

impl RowFile {
    fn create(path: PathBuf) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("MCP directory: can't make {}: {e}", dir.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("MCP directory: can't write {}: {e}", path.display()))?;
        Ok(Self { path, file: Mutex::new(file) })
    }

    fn append(&self, row: &Row) -> Result<RowAt, String> {
        let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
        let mut f = self.file.lock().map_err(|_| "MCP directory: row file lock poisoned".to_string())?;
        let offset = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        if offset + bytes.len() as u64 > MAX_ROW_FILE {
            return Err("MCP directory: the row file is full".into());
        }
        f.write_all(&bytes).map_err(|e| format!("MCP directory: can't write a row: {e}"))?;
        Ok(RowAt { offset: offset as u32, len: bytes.len() as u32 })
    }

    fn read(&self, at: RowAt) -> Option<Row> {
        let mut buf = vec![0u8; at.len as usize];
        {
            let mut f = self.file.lock().ok()?;
            f.seek(SeekFrom::Start(u64::from(at.offset))).ok()?;
            f.read_exact(&mut buf).ok()?;
        }
        serde_json::from_slice(&buf).map_err(|e| tracing::warn!(error = %e, "MCP directory: a row didn't read back")).ok()
    }
}

impl Drop for RowFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The copy itself: every one-click entry's latest version, sorted by registry name.
#[derive(Debug)]
pub struct Copy {
    items: Vec<Item>,
    rows: Arc<RowFile>,
    /// When the sync that made this copy started (the next `updated_since`).
    pub synced_from: Option<SystemTime>,
}

impl Copy {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Registry names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|i| i.key.name())
    }
    /// The row of one entry, read back from disk.
    pub fn row(&self, name: &str) -> Option<Row> {
        let at = self.items.binary_search_by(|i| i.key.name().cmp(name)).ok()?;
        self.rows.read(self.items[at].row)
    }
}

/// One entry a sync page brought: a new or changed entry, or one that's gone (deleted, or no
/// longer one click).
enum Change {
    Put(Item),
    Gone(String),
}

impl Change {
    fn name(&self) -> &str {
        match self {
            Change::Put(i) => i.key.name(),
            Change::Gone(n) => n,
        }
    }
}

/// `previous` (sorted by name) with `changes` (in arrival order) applied: the last change to a
/// name wins. Sorted by name; at most `max` entries (the rest, last in name order, are dropped).
fn merge(previous: &[Item], mut changes: Vec<Change>, max: usize) -> Vec<Item> {
    // A stable sort keeps arrival order among one name's changes; keep each name's last one.
    changes.sort_by(|a, b| a.name().cmp(b.name()));
    let mut last: Vec<Change> = Vec::with_capacity(changes.len());
    for c in changes {
        match last.last_mut() {
            Some(l) if l.name() == c.name() => *l = c,
            _ => last.push(c),
        }
    }
    let mut out = Vec::with_capacity(previous.len() + last.len());
    let (mut old, mut new) = (previous.iter().peekable(), last.into_iter().peekable());
    loop {
        let take_new = match (old.peek(), new.peek()) {
            (None, None) => break,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (Some(o), Some(n)) => match o.key.name().cmp(n.name()) {
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => {
                    old.next();
                    true
                }
            },
        };
        if take_new {
            if let Some(Change::Put(i)) = new.next() {
                out.push(i);
            }
        } else if let Some(o) = old.next() {
            out.push(o.clone());
        }
    }
    if out.len() > max {
        tracing::warn!(kept = max, dropped = out.len() - max, "MCP directory: more entries than the safety stop; the rest are left out");
        out.truncate(max);
    }
    out
}

/// Moves every key's text into one string the whole copy shares (A2 round 3). A sync builds each
/// key on its own, between the pages and rows it parses; 75,000 small strings left among that
/// freed memory kept ~12 MB of it from going back to the system, and each cost its own header.
/// Items keep their order. Left as they are if the text wouldn't fit 32-bit starts (no real
/// registry is near 4 GB of keys).
fn pack(mut items: Vec<Item>) -> Vec<Item> {
    let total: usize = items.iter().map(|i| i.key.all().len()).sum();
    if u32::try_from(total).is_err() {
        return items;
    }
    let mut text = String::with_capacity(total);
    for i in &items {
        text.push_str(i.key.all());
    }
    let text = Arc::new(text);
    let mut start = 0u32;
    for i in &mut items {
        let len = i.key.ends[4];
        i.key.text = text.clone();
        i.key.start = start;
        start += u32::from(len);
    }
    items
}

/// Hands memory the allocator holds but nothing uses back to the system. A full sync parses the
/// whole registry page by page on tokio's worker threads, and glibc keeps what those threads
/// freed in their own arenas: CI measured ~12 MB of it still resident 30 s after a 75,000-entry
/// sync. Elsewhere (macOS, musl) a no-op.
fn give_back_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only walks glibc's own arenas and releases free pages; it takes a
    // plain integer and touches no memory of ours.
    unsafe {
        libc::malloc_trim(0);
    }
}

static ROW_FILES: AtomicUsize = AtomicUsize::new(0);

/// The directory: the copy, once the first sync finished. The registry it syncs from is passed
/// in (`AppState::mcp_registry`, which `KWNT_MCP_REGISTRY_URL` and tests point elsewhere).
#[derive(Default)]
pub struct Directory {
    copy: RwLock<Option<Arc<Copy>>>,
    /// What the background sync needs, from [`Directory::arm`] until the first search takes it
    /// and starts the sync. `None` before arming (tests) and after the sync started.
    armed: Mutex<Option<(Arc<McpRegistry>, CancellationToken)>>,
    /// Where the row file goes (kwnt's data folder); the system temp folder when unset (tests).
    rows_dir: Option<PathBuf>,
}

/// Where a search answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Copy,
    Live,
}

pub struct SearchAnswer {
    pub entries: Vec<Row>,
    pub source: Source,
    /// Neither a copy nor the registry could answer.
    pub unavailable: bool,
}

impl Directory {
    /// A directory whose row file goes in `rows_dir` (kwnt's data folder).
    pub fn new(rows_dir: PathBuf) -> Self {
        Self { rows_dir: Some(rows_dir), ..Self::default() }
    }

    /// The current copy, if the first sync has finished.
    pub fn copy(&self) -> Option<Arc<Copy>> {
        self.copy.read().ok().and_then(|c| c.clone())
    }

    fn set(&self, copy: Copy) {
        if let Ok(mut c) = self.copy.write() {
            *c = Some(Arc::new(copy));
        }
    }

    /// A fresh row file: in kwnt's data folder, or a unique name in the temp folder.
    fn new_row_file(&self) -> Result<RowFile, String> {
        let n = ROW_FILES.fetch_add(1, Ordering::Relaxed);
        let path = match &self.rows_dir {
            Some(dir) => dir.join(format!("mcp-directory-{n}.rows")),
            None => std::env::temp_dir().join(format!("kwnt-mcp-directory-{}-{n}.rows", std::process::id())),
        };
        RowFile::create(path)
    }

    /// Startup: the background sync starts on the first search ([`Directory::search`]), not now.
    pub fn arm(&self, registry: Arc<McpRegistry>, shutdown: CancellationToken) {
        if let Ok(mut a) = self.armed.lock() {
            *a = Some((registry, shutdown));
        }
    }

    /// Starts the armed background sync, once. True when this call started it.
    pub fn start_sync(self: &Arc<Self>) -> bool {
        let Some((registry, shutdown)) = self.armed.lock().ok().and_then(|mut a| a.take()) else { return false };
        self.clone().spawn(registry, shutdown);
        true
    }

    /// Answers from the copy; before the first sync, from the registry's own search. The first
    /// search starts the armed sync.
    pub async fn search(self: &Arc<Self>, registry: &McpRegistry, query: &str) -> SearchAnswer {
        self.start_sync();
        if let Some(copy) = self.copy() {
            let entries = rank(query, &copy.items, MAX_RESULTS).into_iter().filter_map(|i| copy.rows.read(i.row)).collect();
            return SearchAnswer { entries, source: Source::Copy, unavailable: false };
        }
        if normalize(query).chars().count() < MIN_QUERY_CHARS {
            return SearchAnswer { entries: Vec::new(), source: Source::Live, unavailable: false };
        }
        match registry.search(query).await {
            Ok(hits) => {
                let all: Vec<DirEntry> = hits.into_iter().filter_map(DirEntry::new).collect();
                let entries = rank(query, &all, MAX_RESULTS).into_iter().map(|e| e.row.clone()).collect();
                SearchAnswer { entries, source: Source::Live, unavailable: false }
            }
            Err(e) => {
                tracing::info!(error = %e, "MCP directory: no copy yet and the registry search failed");
                SearchAnswer { entries: Vec::new(), source: Source::Live, unavailable: true }
            }
        }
    }

    /// One sync: the whole registry when there's no copy yet, else what changed since the last
    /// one. The copy is replaced only when every page arrived.
    pub async fn sync_once(&self, registry: &McpRegistry) -> Result<usize, String> {
        let started = SystemTime::now();
        let previous = self.copy();
        let since = previous.as_ref().and_then(|c| c.synced_from).map(|t| {
            let t = t.checked_sub(OVERLAP).unwrap_or(t);
            chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        });
        // An update appends to the copy's own row file (readers only ever read rows written
        // before they ranked); the first sync starts a new one.
        let (base, rows): (&[Item], Arc<RowFile>) = match (&previous, &since) {
            (Some(c), Some(_)) => (&c.items, c.rows.clone()),
            _ => (&[], Arc::new(self.new_row_file()?)),
        };
        let mut changes = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = 0usize;
        for page_no in 0..MAX_PAGES {
            if page_no > 0 {
                tokio::time::sleep(PAGE_PAUSE).await;
            }
            let page = registry.page(cursor.as_deref(), since.as_deref()).await?;
            seen += page.items.len();
            for (name, entry) in page.items {
                match entry.and_then(DirEntry::new) {
                    Some(e) => changes.push(Change::Put(Item { row: rows.append(&e.row)?, key: e.key })),
                    None => changes.push(Change::Gone(name)),
                }
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
        let items = pack(merge(base, changes, MAX_ENTRIES));
        let kept = items.len();
        self.set(Copy { items, rows, synced_from: Some(started) });
        tracing::info!(listed = seen, kept, full = since.is_none(), "MCP directory synced");
        Ok(kept)
    }

    /// Syncs now and then every 6 hours, retrying failures with a growing pause, until shutdown.
    pub fn spawn(self: Arc<Self>, registry: Arc<McpRegistry>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut retry = RETRY_FIRST;
            loop {
                let synced = self.sync_once(&registry).await;
                // The pages, rows and the copy this sync replaced are all freed by now.
                give_back_freed_memory();
                let wait = match synced {
                    Ok(_) => {
                        retry = RETRY_FIRST;
                        SYNC_EVERY
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "MCP directory sync failed; trying again later");
                        let w = retry;
                        retry = (retry * 2).min(RETRY_MAX);
                        w
                    }
                };
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::registry::parse_list;

    const STRIPE: &str = include_str!("../../tests/fixtures/mcp_registry/stripe.json");
    const NOTION: &str = include_str!("../../tests/fixtures/mcp_registry/notion.json");

    fn fixture(body: &str) -> Vec<DirEntry> {
        parse_list(body.as_bytes()).unwrap().into_iter().filter_map(DirEntry::new).collect()
    }

    fn names(query: &str, list: &[DirEntry]) -> Vec<String> {
        rank(query, list, 50).into_iter().map(|e| e.row.registry_name.clone()).collect()
    }

    #[test]
    fn stripe_ranks_stripe_first_and_drops_refused_plans() {
        let list = fixture(STRIPE);
        let ranked = names("stripe", &list);
        assert_eq!(ranked[0], "com.stripe/mcp", "{ranked:?}");
        let top = &rank("stripe", &list, 1)[0];
        assert_eq!(top.publisher, Publisher { label: "stripe.com".into(), kind: "domain" });
        assert_eq!(top.row.title, "Stripe");
        // eu.nordicmcp/stripe's only address is https://nordicmcp.eu/mcp/stripe/{token}: not one-click
        assert!(!ranked.iter().any(|n| n == "eu.nordicmcp/stripe"), "{ranked:?}");
        // mcpb packages aren't installable either
        assert!(!ranked.iter().any(|n| n == "io.github.theluckystrike/stripe-billing"));
        // Look-alikes come after, and an exact name ("stripe") before the rest
        assert_eq!(ranked[1], "com.insightfulpipe/stripe", "{ranked:?}");
        assert!(ranked.len() >= 8, "{ranked:?}");
        // Upper case, spaces and typos
        assert_eq!(names("STRIPE ", &list)[0], "com.stripe/mcp");
        assert_eq!(names("strpie", &list)[0], "com.stripe/mcp");
    }

    #[test]
    fn notion_ranks_notion_first_and_smithery_last() {
        let list = fixture(NOTION);
        let ranked = names("notion", &list);
        assert_eq!(ranked[0], "com.notion/mcp", "{ranked:?}");
        assert_eq!(ranked.last().map(String::as_str), Some("ai.smithery/smithery-notion"), "{ranked:?}");
        assert_eq!(names("notoin", &list)[0], "com.notion/mcp");
        // Remote before stdio among equal matches
        let remote_at = ranked.iter().position(|n| n == "com.insightfulpipe/notion").unwrap();
        let stdio_at = ranked.iter().position(|n| n == "com.foreverworks/notion").unwrap();
        assert!(remote_at < stdio_at, "{ranked:?}");
    }

    #[test]
    fn placeholder_addresses_go_last_even_with_a_package() {
        let body = serde_json::json!({"servers": [
            {"server": {"name": "io.github.a/weather-tenant", "title": "Weather", "description": "d", "version": "1",
                "remotes": [{"type": "streamable-http", "url": "https://{tenant}.weather.example/mcp"}],
                "packages": [{"registryType": "npm", "identifier": "weather-mcp", "version": "1.0.0", "transport": {"type": "stdio"}}]}},
            {"server": {"name": "io.github.b/weather-x", "description": "d", "version": "1",
                "packages": [{"registryType": "npm", "identifier": "weather-x", "version": "1.0.0", "transport": {"type": "stdio"}}]}},
        ]});
        let list = fixture(&body.to_string());
        assert_eq!(names("weather", &list), ["io.github.b/weather-x", "io.github.a/weather-tenant"]);
    }

    #[test]
    fn short_queries_aliases_and_publishers() {
        let list = fixture(STRIPE);
        assert!(names("s", &list).is_empty());
        assert!(names("--", &list).is_empty());
        assert_eq!(publisher_of("io.github.Alice/x"), Publisher { label: "github.com/alice".into(), kind: "github" });
        assert_eq!(publisher_of("app.vercel.stripecheckup/x").label, "stripecheckup.vercel.app");
        assert_eq!(normalize("Google-Sheets É"), "googlesheetse");
        assert_eq!(osa_bounded("notoin", "notion", 2), 1);
        assert_eq!(osa_bounded("gmial", "gmail", 2), 1);
        assert_eq!(osa_bounded("kitten", "sitting", 5), 3);
        assert_eq!(osa_bounded("ab", "ba", 2), 1);
        assert_eq!(osa_bounded("abc", "abcdefgh", 2), 3);
        assert_eq!(osa_bounded("abcdef", "uvwxyz", 1), 2, "stops early, past the limit");
        assert_eq!(osa_bounded("café", "cafe", 2), 1);
        assert_eq!(osa_bounded(&"a".repeat(41), &"a".repeat(41), 2), 3, "too long to compare");
        assert_eq!((typo_budget(3), typo_budget(4), typo_budget(8), typo_budget(41)), (0, 1, 2, 0));
        for (a, b) in [("abcd", "abcd"), ("abcd", "abxd"), ("abcd", "bacd"), ("abcd", "abdc"), ("strpie", "stripe"), ("cafe", "café"), ("abcd", "abcde"), ("abcd", "xbcy"), ("abcd", "badc"), ("abcd", "acdb"), ("abcd", "dbca")] {
            assert_eq!(one_edit_same_length(a, b), osa_bounded(a, b, 1) <= 1 && char_count(a) == char_count(b), "{a} {b}");
        }
        assert_eq!((char_count("café"), char_count("cafe")), (4, 4));
        for (a, b) in [("gmail", "gmial"), ("gmail", "gmai"), ("gmail", "gmaill"), ("gmail", "xgmail"), ("gmail", "gmal"), ("gmail", "gmali"), ("gmail", "mail"), ("gmail", "gmailxx"), ("gmail", "gamil"), ("gmail", "gnail"), ("abcd", "acbd"), ("abcd", "abc"), ("abc", "ab")] {
            for (x, y) in [(a, b), (b, a)] {
                assert_eq!(within(x, char_count(x), y, 1), osa_bounded(x, y, 1) <= 1, "{x} {y}");
                assert_eq!(within(x, char_count(x), y, 2), osa_bounded(x, y, 2) <= 2, "{x} {y}");
            }
        }
        assert!(within("abc", 3, "abc", 0) && !within("abc", 3, "abd", 0));
        let pg = fixture(
            &serde_json::json!({"servers": [{"server": {"name": "io.github.x/postgresql-tools", "description": "d", "version": "1",
                "remotes": [{"type": "streamable-http", "url": "https://pg.example/mcp"}]}}]})
            .to_string(),
        );
        assert_eq!(rank("pg", &pg, 5).len(), 1);
        assert_eq!(rank("pg", &pg, 5)[0].matches("pg"), Some(Match::Alias));
    }

    /// Remote entries named like the real registry's: `(registry name, title)`.
    fn remotes(list: &[(&str, &str)]) -> Vec<DirEntry> {
        let servers: Vec<_> = list
            .iter()
            .enumerate()
            .map(|(i, (name, title))| {
                serde_json::json!({"server": {"name": name, "title": title, "description": "d", "version": "1",
                    "remotes": [{"type": "streamable-http", "url": format!("https://mcp{i}.example.com/mcp")}]}})
            })
            .collect();
        fixture(&serde_json::json!({ "servers": servers }).to_string())
    }

    #[test]
    fn a_publisher_a_typo_away_never_beats_a_closer_match() {
        // From the real registry (Nov 13): stride.info and scripe.io are one letter from stripe
        let list = remotes(&[
            ("info.stride/stride", "Stride"),
            ("io.scripe/mcp", "Scripe"),
            ("com.insightfulpipe/stripe", "Stripe"),
            ("com.stripe/mcp", "Stripe"),
            ("io.github.someone/stripe-billing", "Stripe billing"),
        ]);
        let ranked = names("stripe", &list);
        assert_eq!(&ranked[..3], ["com.stripe/mcp", "com.insightfulpipe/stripe", "io.github.someone/stripe-billing"], "{ranked:?}");
        // A typo in the query: the publisher a typo away goes first among the typo matches
        assert_eq!(names("strpie", &list)[0], "com.stripe/mcp");

        let list = remotes(&[
            ("fr.gentry/logement", "Gentry logement"),
            ("fyi.entry/entry", "Entry"),
            ("io.github.getsentry/sentry-mcp", "Sentry"),
        ]);
        assert_eq!(names("sentry", &list)[0], "io.github.getsentry/sentry-mcp");

        // Among typo matches, the name itself a typo away comes before a word in a longer title
        let list = remotes(&[("ai.fgac/fgac", "Fgac for gmail and drive"), ("ai.waystation/gmail", "Gmail")]);
        assert_eq!(names("gmial", &list), ["ai.waystation/gmail", "ai.fgac/fgac"]);
    }

    #[test]
    fn github_publishers_and_subdomains() {
        let list = remotes(&[
            ("com.robinsonaisystems/github", "GitHub"),
            ("io.github.github/github-mcp-server", "GitHub"),
            ("io.ddx.pg/archive", "Archive"),
            ("io.github.x/postgresql-tools", "Postgres tools"),
            ("app.vercel.stripecheckup/x", "Checkup"),
        ]);
        assert_eq!(names("github", &list)[0], "io.github.github/github-mcp-server");
        assert_eq!(names("getsentry", &remotes(&[("io.github.getsentry/sentry-mcp", "Sentry")])).len(), 1);
        // pg.ddx.io's owner is ddx.io, not "pg": the alias for PostgreSQL comes first
        assert_eq!(names("pg", &list), ["io.github.x/postgresql-tools", "io.ddx.pg/archive"]);
        // stripecheckup.vercel.app isn't Vercel's own
        assert_eq!(list[4].key.publisher_key(), None);
        assert_eq!(list[1].key.publisher_key(), Some("github"));
        assert_eq!(list[0].key.publisher_key(), Some("robinsonaisystems"));
    }

    #[test]
    fn typos_are_skipped_when_closer_matches_fill_the_list() {
        let mut named: Vec<(String, String)> = (0..20).map(|i| (format!("com.acme{i}/stripe-tools"), format!("Stripe tools {i}"))).collect();
        named.push(("info.stride/stride".into(), "Stride".into()));
        let list = remotes(&named.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
        let top = rank("stripe", &list, MAX_RESULTS);
        assert_eq!(top.len(), MAX_RESULTS);
        assert!(top.iter().all(|e| e.row.registry_name.starts_with("com.acme")));
        let ranked = names("stripe", &list);
        assert_eq!((ranked.len(), ranked.last().map(String::as_str)), (21, Some("info.stride/stride")), "{ranked:?}");
        // With room left, the typo match is there, last
        assert_eq!(names("stride", &list)[0], "info.stride/stride");
        assert_eq!(rank("stripe", &list[15..], MAX_RESULTS).last().map(|e| e.row.registry_name.as_str()), Some("info.stride/stride"));
    }

    /// perf-idle-memory A2: an entry of the copy is a packed key plus where its row sits.
    #[test]
    fn a_kept_entry_is_small_and_its_key_reads_back() {
        assert!(std::mem::size_of::<Item>() <= 40, "{}", std::mem::size_of::<Item>());
        let list = fixture(STRIPE);
        let stripe = list.iter().find(|e| e.row.registry_name == "com.stripe/mcp").unwrap();
        let k = &stripe.key;
        assert_eq!(k.name(), "com.stripe/mcp");
        assert_eq!((k.title_norm(), k.name_norm(), k.slug_norm()), ("stripe", "comstripemcp", "mcp"));
        assert_eq!(k.words().collect::<Vec<_>>(), ["stripe", "mcp"]);
        assert_eq!((k.publisher_norm(), k.publisher_key()), ("stripecom", Some("stripe")));
        assert_eq!(k.parts().collect::<Vec<_>>(), ["com", "stripe", "mcp"]);
        assert!(k.remote && !k.demoted);
        // What matching keeps is a small part of the entry: no description, plan or inputs
        assert!(k.all().len() < 64, "{}", k.all().len());
        let full = serde_json::to_vec(&stripe.row).unwrap().len();
        assert!(k.all().len() * 2 < full, "{} vs {full}", k.all().len());
    }

    fn parts_of(k: &Key) -> Vec<String> {
        let mut v: Vec<String> = [k.name(), k.title_norm(), k.name_norm(), k.slug_norm(), k.publisher_norm()].map(String::from).into();
        v.extend(k.words().map(String::from));
        v.push(k.publisher_key().unwrap_or("-").to_string());
        v.push(format!("{} {}", k.demoted, k.remote));
        v
    }

    /// The slug and the publisher key aren't stored; what they read back must be what they were.
    #[test]
    fn slug_and_publisher_key_read_back_from_the_name_and_label() {
        for name in [
            "com.stripe/mcp",
            "io.github.getsentry/sentry-mcp",
            "io.github.Ünï.Cödé-Org/Café-Tools_2",
            "com.Exämple/Ñandú",
            "io.vercel.stripecheckup/x",
            "plainname",
            "com.x/a/b/İstanbul",
        ] {
            let p = publisher_of(name);
            let k = Key::new(name, "Some Title", &p, false, true).unwrap_or_else(|| panic!("{name} kept"));
            let slug = name.rsplit('/').next().unwrap();
            assert_eq!(k.slug_norm(), normalize(slug), "{name}");
            assert_eq!(k.publisher_key(), publisher_key(&p).as_deref(), "{name}");
            assert_eq!(k.publisher_norm(), normalize(&p.label), "{name}");
        }
    }

    /// A copy's keys share one string and match exactly as they did one by one.
    #[test]
    fn packed_keys_share_one_string_and_read_back_the_same() {
        let mut list = fixture(STRIPE);
        list.extend(fixture(NOTION));
        let items: Vec<Item> = list.iter().enumerate().map(|(n, e)| Item { key: e.key.clone(), row: RowAt { offset: n as u32, len: 1 } }).collect();
        let before: Vec<Vec<String>> = items.iter().map(|i| parts_of(&i.key)).collect();
        let ranked: Vec<u32> = rank("stripe", &items, MAX_RESULTS).iter().map(|i| i.row.offset).collect();
        let packed = pack(items);
        assert_eq!(packed.iter().map(|i| parts_of(&i.key)).collect::<Vec<_>>(), before);
        assert!(packed.windows(2).all(|w| Arc::ptr_eq(&w[0].key.text, &w[1].key.text)), "one string for the copy");
        assert_eq!(packed[0].key.text.len(), packed.iter().map(|i| i.key.all().len()).sum::<usize>());
        assert_eq!(rank("stripe", &packed, MAX_RESULTS).iter().map(|i| i.row.offset).collect::<Vec<_>>(), ranked);
        // A second pack (an update sync) copies the text again and drops nothing
        let again = pack(packed.clone());
        assert_eq!(again.iter().map(|i| parts_of(&i.key)).collect::<Vec<_>>(), before);
        assert!(!Arc::ptr_eq(&again[0].key.text, &packed[0].key.text));
        drop(packed);
        give_back_freed_memory();
        assert_eq!(again.iter().map(|i| parts_of(&i.key)).collect::<Vec<_>>(), before);
    }

    fn item(name: &str, n: u32) -> Item {
        let p = publisher_of(name);
        Item { key: Key::new(name, name, &p, false, true).unwrap(), row: RowAt { offset: n, len: 1 } }
    }

    fn put(name: &str, n: u32) -> Change {
        Change::Put(item(name, n))
    }

    #[test]
    fn merge_keeps_name_order_and_the_last_change_wins() {
        let previous = vec![item("a.x/one", 0), item("b.x/two", 1), item("c.x/three", 2)];
        let changes = vec![
            put("d.x/four", 10),
            Change::Gone("b.x/two".into()),
            put("a.x/one", 11),
            put("a.x/one", 12),
            put("e.x/five", 13),
            Change::Gone("e.x/five".into()),
            Change::Gone("zz.x/never-there".into()),
        ];
        let out = merge(&previous, changes, 100);
        let got: Vec<(&str, u32)> = out.iter().map(|i| (i.key.name(), i.row.offset)).collect();
        assert_eq!(got, [("a.x/one", 12), ("c.x/three", 2), ("d.x/four", 10)]);
        // A gone entry that comes back later in the same sync is kept
        let out = merge(&[], vec![put("a.x/one", 1), Change::Gone("a.x/one".into()), put("a.x/one", 3)], 100);
        assert_eq!(out.iter().map(|i| i.row.offset).collect::<Vec<_>>(), [3]);
    }

    /// The registry had 74,866 entries on Nov 26 and the old 60,000 stop cut off the end of the
    /// alphabet. Every entry is kept now; the stop only guards a registry that never ends.
    #[test]
    fn more_than_sixty_thousand_entries_are_all_kept() {
        let changes: Vec<Change> = (0..75_000u32).map(|i| put(&format!("io.github.u{i:05}/tool"), i)).collect();
        let out = merge(&[], changes, MAX_ENTRIES);
        assert_eq!(out.len(), 75_000);
        assert_eq!(out.last().map(|i| i.key.name()), Some("io.github.u74999/tool"));
        assert!(out.windows(2).all(|w| w[0].key.name() < w[1].key.name()));
        assert_eq!(merge(&out, vec![], 10).len(), 10, "the safety stop still holds");
    }

    #[test]
    fn rows_wait_on_disk_and_the_file_goes_with_the_copy() {
        let dir = std::env::temp_dir().join(format!("kwnt-dir-rows-test-{}", std::process::id()));
        let d = Directory::new(dir.clone());
        let rows = d.new_row_file().unwrap();
        let path = rows.path.clone();
        assert!(path.starts_with(&dir));
        let list = fixture(NOTION);
        let at: Vec<RowAt> = list.iter().map(|e| rows.append(&e.row).unwrap()).collect();
        for (e, at) in list.iter().zip(&at).rev() {
            assert_eq!(rows.read(*at).as_ref(), Some(&e.row));
        }
        assert!(rows.read(RowAt { offset: at[0].offset, len: at[0].len - 1 }).is_none(), "a cut row isn't half read");
        drop(rows);
        assert!(!path.exists(), "the row file is removed with its copy");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ten_thousand_entries_answer_fast() {
        // Varied names, like the real copy (about 6,000 one-click entries on Nov 13)
        let mut seed = 7u64;
        let mut word = |len: usize| -> String {
            (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    char::from(b'a' + ((seed >> 33) % 26) as u8)
                })
                .collect()
        };
        let named: Vec<(String, String)> =
            (0..12_000).map(|i| (format!("io.github.{}/{}-mcp", word(8), word(3 + i % 9)), format!("{} {} tools", word(6), word(7)))).collect();
        let list = remotes(&named.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
        assert_eq!(list.len(), 12_000);
        let started = std::time::Instant::now();
        for q in ["st", "stripe", "strpie", "zqxw", "zzzzzzzz", "billing tools", "pg"] {
            rank(q, &list, MAX_RESULTS);
        }
        // Seven searches; a release build takes a few ms each. Debug builds are many times slower,
        // so this only guards against something going badly wrong.
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }
}
