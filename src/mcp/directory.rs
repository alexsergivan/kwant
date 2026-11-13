//! The public MCP directory as people search it ("Find a tool", mcp-finder): a local copy of the
//! official MCP Registry's latest versions, synced in the background, and the matching and
//! ranking that turn a few typed letters into up to 12 rows in well under 50 ms.
//!
//! The first sync pages through the whole registry; after that kwnt asks only for what changed
//! (`updated_since`) every 6 hours. Until the first sync has finished, a search goes to the
//! registry's own (name-only) search instead. Only entries kwnt can set up in one click are kept.
//!
//! Ranking: entries from Smithery's proxy or with a `{placeholder}` address go last; then a
//! publisher whose proven domain is the query (`stripe` → stripe.com); then how well the name
//! matches (starts with > a word starts with > alias > a part of the registry name > contains >
//! typo); then an exact name or title; then remote before stdio; then registry order.

use super::registry::{publisher_name, McpRegistry, RegistryEntry, GENERIC_SERVER_PARTS};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
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
/// Safety stops for a registry that never ends.
const MAX_PAGES: usize = 1000;
const MAX_ENTRIES: usize = 60_000;
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

/// One entry of the copy, with what matching needs worked out once.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub entry: RegistryEntry,
    pub publisher: Publisher,
    /// The registry's title, else a name made from its registry name.
    pub title: String,
    title_norm: String,
    slug_norm: String,
    name_norm: String,
    words: Vec<String>,
    /// Dot and slash separated parts of the registry name.
    parts: Vec<String>,
    /// The leftmost label of a domain publisher (`stripe` for stripe.com).
    publisher_key: Option<String>,
    demoted: bool,
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
        entry.plan.as_ref().ok()?;
        let publisher = publisher_of(&entry.name);
        let title = display_title(&entry);
        let slug = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        let mut ws = words(&title);
        for w in words(slug) {
            if !ws.contains(&w) {
                ws.push(w);
            }
        }
        let publisher_key = (publisher.kind == "domain").then(|| normalize(publisher.label.split('.').next().unwrap_or("")));
        Some(Self {
            title_norm: normalize(&title),
            slug_norm: normalize(slug),
            name_norm: normalize(&entry.name),
            words: ws,
            parts: entry.name.split(['.', '/']).map(normalize).filter(|p| !p.is_empty()).collect(),
            demoted: entry.name.to_ascii_lowercase().starts_with("ai.smithery/") || entry.placeholder_url,
            publisher_key,
            publisher,
            title,
            entry,
        })
    }

    fn is_remote(&self) -> bool {
        self.entry.plan.as_ref().is_ok_and(|p| p.transport == "http")
    }

    fn direct_match(&self, q: &str) -> Option<Match> {
        if self.title_norm.starts_with(q) || self.slug_norm.starts_with(q) || self.publisher_key.as_deref().is_some_and(|k| k.starts_with(q)) {
            Some(Match::Prefix)
        } else if self.words.iter().any(|w| w.starts_with(q)) {
            Some(Match::WordPrefix)
        } else {
            None
        }
    }

    /// How `q` (already normalised, 2+ characters) matches this entry, if at all.
    pub fn matches(&self, q: &str) -> Option<Match> {
        if let Some(m) = self.direct_match(q) {
            return Some(m);
        }
        if let Some((_, canonical)) = ALIASES.iter().find(|(a, _)| *a == q) {
            if self.direct_match(&normalize(canonical)).is_some() {
                return Some(Match::Alias);
            }
        }
        if self.parts.iter().any(|p| p.starts_with(q)) {
            return Some(Match::NamePart);
        }
        if self.title_norm.contains(q) || self.name_norm.contains(q) {
            return Some(Match::Contains);
        }
        let allowed = typo_budget(q);
        if allowed > 0 {
            let n = q.chars().count();
            let whole = |w: &String| osa_distance(q, w) <= allowed;
            let start = |w: &String| w.chars().count() > n && osa_distance(q, &w.chars().take(n).collect::<String>()) <= allowed;
            let all = || self.words.iter().chain([&self.title_norm, &self.slug_norm]);
            if all().any(whole) {
                return Some(Match::Typo);
            }
            if all().any(start) {
                return Some(Match::TypoStart);
            }
        }
        None
    }

    fn exact(&self, q: &str) -> bool {
        self.title_norm == q || self.slug_norm == q
    }
}

/// 1 edit from 4 letters, 2 from 8 (autocomplete.md §4).
fn typo_budget(q: &str) -> usize {
    match q.chars().count() {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// Edits (insert, delete, substitute, swap two neighbours) between two short strings.
pub fn osa_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The best `limit` entries for `query`, in order. `entries` is in registry order.
pub fn rank<'a>(query: &str, entries: impl IntoIterator<Item = &'a DirEntry>, limit: usize) -> Vec<&'a DirEntry> {
    let q = normalize(query);
    if q.chars().count() < MIN_QUERY_CHARS {
        return Vec::new();
    }
    let budget = typo_budget(&q);
    let mut hits: Vec<_> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(order, e)| {
            let m = e.matches(&q)?;
            let publisher = e.publisher_key.as_deref().is_some_and(|k| k == q || (budget > 0 && osa_distance(k, &q) <= budget))
                || normalize(&e.publisher.label) == q;
            Some(((e.demoted, !publisher, m, !e.exact(&q), !e.is_remote(), order), e))
        })
        .collect();
    hits.sort_by_key(|h| h.0);
    hits.into_iter().take(limit).map(|(_, e)| e).collect()
}

/// The copy itself: every one-click entry's latest version, by registry name.
#[derive(Debug, Default)]
pub struct Copy {
    pub entries: BTreeMap<String, DirEntry>,
    /// When the sync that made this copy started (the next `updated_since`).
    pub synced_from: Option<SystemTime>,
}

/// The directory: the copy, once the first sync finished. The registry it syncs from is passed
/// in (`AppState::mcp_registry`, which `KWNT_MCP_REGISTRY_URL` and tests point elsewhere).
#[derive(Default)]
pub struct Directory {
    copy: RwLock<Option<Arc<Copy>>>,
}

/// Where a search answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Copy,
    Live,
}

pub struct SearchAnswer {
    pub entries: Vec<DirEntry>,
    pub source: Source,
    /// Neither a copy nor the registry could answer.
    pub unavailable: bool,
}

impl Directory {
    /// The current copy, if the first sync has finished.
    pub fn copy(&self) -> Option<Arc<Copy>> {
        self.copy.read().ok().and_then(|c| c.clone())
    }

    fn set(&self, copy: Copy) {
        if let Ok(mut c) = self.copy.write() {
            *c = Some(Arc::new(copy));
        }
    }

    /// Answers from the copy; before the first sync, from the registry's own search.
    pub async fn search(&self, registry: &McpRegistry, query: &str) -> SearchAnswer {
        if let Some(copy) = self.copy() {
            let entries = rank(query, copy.entries.values(), MAX_RESULTS).into_iter().cloned().collect();
            return SearchAnswer { entries, source: Source::Copy, unavailable: false };
        }
        if normalize(query).chars().count() < MIN_QUERY_CHARS {
            return SearchAnswer { entries: Vec::new(), source: Source::Live, unavailable: false };
        }
        match registry.search(query).await {
            Ok(hits) => {
                let all: Vec<DirEntry> = hits.into_iter().filter_map(DirEntry::new).collect();
                let entries = rank(query, all.iter(), MAX_RESULTS).into_iter().cloned().collect();
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
        let mut entries = match (&previous, &since) {
            (Some(c), Some(_)) => c.entries.clone(),
            _ => BTreeMap::new(),
        };
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
                    Some(e) if entries.len() < MAX_ENTRIES || entries.contains_key(&name) => {
                        entries.insert(name, e);
                    }
                    Some(_) => {}
                    None => {
                        entries.remove(&name);
                    }
                }
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
        let kept = entries.len();
        self.set(Copy { entries, synced_from: Some(started) });
        tracing::info!(listed = seen, kept, full = since.is_none(), "MCP directory synced");
        Ok(kept)
    }

    /// Syncs now and then every 6 hours, retrying failures with a growing pause, until shutdown.
    pub fn spawn(self: Arc<Self>, registry: Arc<McpRegistry>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut retry = RETRY_FIRST;
            loop {
                let wait = match self.sync_once(&registry).await {
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
        rank(query, list.iter(), 50).into_iter().map(|e| e.entry.name.clone()).collect()
    }

    #[test]
    fn stripe_ranks_stripe_first_and_drops_refused_plans() {
        let list = fixture(STRIPE);
        let ranked = names("stripe", &list);
        assert_eq!(ranked[0], "com.stripe/mcp", "{ranked:?}");
        let top = &rank("stripe", list.iter(), 1)[0];
        assert_eq!(top.publisher, Publisher { label: "stripe.com".into(), kind: "domain" });
        assert_eq!(top.title, "Stripe");
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
        assert_eq!(osa_distance("notoin", "notion"), 1);
        assert_eq!(osa_distance("gmial", "gmail"), 1);
        assert_eq!(osa_distance("abc", "abcdefgh"), 3);
        let pg = fixture(
            &serde_json::json!({"servers": [{"server": {"name": "io.github.x/postgresql-tools", "description": "d", "version": "1",
                "remotes": [{"type": "streamable-http", "url": "https://pg.example/mcp"}]}}]})
            .to_string(),
        );
        assert_eq!(rank("pg", pg.iter(), 5).len(), 1);
        assert_eq!(rank("pg", pg.iter(), 5)[0].matches("pg"), Some(Match::Alias));
    }

    #[test]
    fn ten_thousand_entries_answer_fast() {
        let base = fixture(STRIPE);
        let mut big = Vec::new();
        for i in 0..700 {
            for e in &base {
                let mut e = e.clone();
                e.entry.name = format!("{}{i}", e.entry.name);
                big.push(e);
            }
        }
        assert!(big.len() > 9000);
        let started = std::time::Instant::now();
        for q in ["st", "stripe", "strpie", "zzzzzzzz", "billing"] {
            rank(q, big.iter(), MAX_RESULTS);
        }
        // Five searches; release builds take a few ms each. Debug builds are slower, so this
        // only guards against something going badly wrong.
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }
}
