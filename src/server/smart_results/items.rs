//! Top-level lists: metrics, findings, cards, timeline, checklist and facts, and the steps block
//! every steps variant shares (plan §A.4). Every value is a slice of the item as written.

use super::chart::{metric_value, when_like};
use super::selector::{is_task, words, List};
use super::validate::has_html;
use super::{
    Card, Cards, Fact, Facts, Finding, Findings, Metric, Metrics, Provenance, Ref, Severity, Source, Step, Steps,
    Variant,
};

fn source(l: &List) -> Source {
    Source::list(l.index, l.ordered)
}

/// Items no selector reads: empty, over 300 characters or with HTML in them.
fn plain_items(l: &List) -> bool {
    l.items.iter().all(|t| !t.is_empty() && t.chars().count() <= 300 && !has_html(t))
}

/// A label and the text after it.
struct Labeled<'a> {
    label: &'a str,
    rest: &'a str,
    bold: bool,
    /// `:`, or `–` / `—` for a dash.
    sep: char,
}

/// `**Label:** rest`, `**Label**: rest`, `**Label** – rest`, `Label: rest`, `Label – rest`.
fn labeled(item: &str) -> Option<Labeled<'_>> {
    let (label, rest, bold, sep) = if let Some(inner) = item.strip_prefix("**") {
        let close = inner.find("**")?;
        let (label, after) = (&inner[..close], &inner[close + 2..]);
        if let Some(l) = label.strip_suffix(':') {
            (l, after.strip_prefix(' ')?, true, ':')
        } else if let Some(r) = after.strip_prefix(':') {
            (label, r.strip_prefix(' ')?, true, ':')
        } else if let Some(r) = after.strip_prefix(" – ").or_else(|| after.strip_prefix(" - ")) {
            (label, r, true, '–')
        } else if let Some(r) = after.strip_prefix(" — ") {
            (label, r, true, '—')
        } else {
            return None;
        }
    } else {
        let at = [": ", " – ", " — "].iter().filter_map(|sep| item.find(sep).map(|i| (i, *sep))).min()?;
        let (i, sep) = at;
        let label = &item[..i];
        if label.contains(['*', '[', ']', '`', '_']) || label.split_whitespace().count() > 5 {
            return None;
        }
        (label, &item[i + sep.len()..], false, sep.trim().chars().next().unwrap_or(':'))
    };
    let label = label.trim();
    let rest = rest.trim();
    if label.is_empty() || rest.is_empty() || label.chars().count() > 60 || !label.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(Labeled { label, rest, bold, sep })
}

/// Intro or heading words that make "**Label:** number" lines a list of things to get, not key
/// numbers.
const NOT_METRICS: [&str; 12] =
    ["ingredients", "ingredient", "recipe", "shopping", "groceries", "packing", "pack", "bring", "need", "serves", "schedule", "agenda"];

fn lead_words(l: &List) -> Vec<String> {
    let mut w = l.intro.map(words).unwrap_or_default();
    w.extend(l.heading.map(words).unwrap_or_default());
    w
}

/// 2–6 items, each "**Label:** value" or "Label: value" whose value starts with a number or a
/// currency; the rest of the item is the note.
pub(super) fn metrics(l: &List) -> Option<Metrics> {
    if !(2..=6).contains(&l.items.len()) || !plain_items(l) || l.items.iter().any(|t| is_task(t)) {
        return None;
    }
    if lead_words(l).iter().any(|w| NOT_METRICS.contains(&w.as_str())) {
        return None;
    }
    let mut items = Vec::new();
    for (i, t) in l.items.iter().enumerate() {
        let x = labeled(t).filter(|x| x.sep == ':')?;
        if when_like(x.label) || x.label.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let v = metric_value(x.rest)?;
        items.push(Metric {
            id: format!("m{i}"),
            label: x.label.to_string(),
            value_text: v.text.to_string(),
            value: v.value,
            unit: v.unit.map(str::to_string),
            note: v.note.map(str::to_string),
        });
    }
    Some(Metrics { id: "b0".into(), title: None, source: source(l), items, provenance: Provenance::reply_text() })
}

/// 3–12 items, each "**Label:** value", more than half of them not starting with a number.
pub(super) fn facts(l: &List) -> Option<Facts> {
    if !(3..=12).contains(&l.items.len()) || !plain_items(l) || l.items.iter().any(|t| is_task(t)) {
        return None;
    }
    let mut items = Vec::new();
    let mut numeric = 0;
    for (i, t) in l.items.iter().enumerate() {
        let x = labeled(t).filter(|x| x.bold && x.sep == ':')?;
        if metric_value(x.rest).is_some() {
            numeric += 1;
        }
        items.push(Fact { id: format!("f{i}"), label: x.label.to_string(), value: x.rest.to_string() });
    }
    if numeric * 2 >= items.len() {
        return None;
    }
    Some(Facts { id: "b0".into(), title: None, source: source(l), items, provenance: Provenance::reply_text() })
}

/// 3–12 items, each `[Title](https://…)` or `**Title**`, then " — ", " – " or " - " (or ": "
/// after a link), then a description of at least 10 characters. `**Title:** text` reads as facts.
pub(super) fn cards(l: &List) -> Option<Cards> {
    if !(3..=12).contains(&l.items.len()) || !plain_items(l) || l.items.iter().any(|t| is_task(t)) {
        return None;
    }
    let mut items = Vec::new();
    for (i, t) in l.items.iter().enumerate() {
        let (title, url, rest) = card_head(t)?;
        let mut description = None;
        for sep in [" — ", " – ", " - "] {
            if let Some(d) = rest.strip_prefix(sep) {
                description = Some(d);
            }
        }
        if url.is_some() {
            if let Some(d) = rest.strip_prefix(": ") {
                description = Some(d);
            }
        }
        let description = description?.trim();
        if description.chars().count() < 10 || title.trim().is_empty() || title.chars().count() > 200 {
            return None;
        }
        items.push(Card {
            id: format!("k{i}"),
            title: title.to_string(),
            url: url.map(str::to_string),
            description: description.to_string(),
            meta: Vec::new(),
        });
    }
    Some(Cards { id: "b0".into(), title: None, source: source(l), items, provenance: Provenance::reply_text() })
}

/// The item's title, its link (https only) and the text after them.
fn card_head(t: &str) -> Option<(&str, Option<&str>, &str)> {
    // `**[Title](url)**` reads like `[Title](url)`
    let (inner, wrapped) = match t.strip_prefix("**").filter(|r| r.starts_with('[')) {
        Some(r) => (r, true),
        None => (t, false),
    };
    if let Some(r) = inner.strip_prefix('[') {
        let close = r.find("](")?;
        let title = &r[..close];
        let after = &r[close + 2..];
        let end = after.find(')')?;
        let url = &after[..end];
        let mut rest = &after[end + 1..];
        if wrapped {
            rest = rest.strip_prefix("**")?;
        }
        if title.contains(['[', ']']) || !url.starts_with("https://") || url.len() <= 8 {
            return None;
        }
        if url.contains(|c: char| c.is_whitespace() || matches!(c, '(' | '<' | '>' | '"' | '\'' | '`')) {
            return None;
        }
        // Link text that looks like another address would hide where the link goes
        let looks_like_url = title.contains("://") || title.trim_start().to_lowercase().starts_with("www.");
        if looks_like_url && title.trim() != url {
            return None;
        }
        return Some((title, Some(url), rest));
    }
    let r = t.strip_prefix("**")?;
    let close = r.find("**")?;
    let title = &r[..close];
    if title.ends_with(':') || title.contains('[') {
        return None;
    }
    Some((title, None, &r[close + 2..]))
}

/// Words in the line before a list, or the heading above it, that say it is a review.
const REVIEW_WORDS: [&str; 15] = [
    "review", "reviewed", "audit", "audited", "finding", "findings", "issue", "issues", "bug", "bugs", "risk", "risks",
    "vulnerability", "vulnerabilities", "problems",
];

/// 2–30 items that each start with a severity token (bold, bracketed or followed by ":"), when
/// the list is introduced as a review or at least one item names a `path:line` in code. An item
/// whose text after the token starts with a number ("High: 18C") keeps the whole list text.
pub(super) fn findings(l: &List) -> Option<Findings> {
    if !(2..=30).contains(&l.items.len()) || !plain_items(l) || l.items.iter().any(|t| is_task(t)) {
        return None;
    }
    let mut items = Vec::new();
    for (i, t) in l.items.iter().enumerate() {
        let (severity, token, text) = severity_lead(t)?;
        let first = text.trim_start_matches(['*', '_', '`']);
        let mut chars = first.chars();
        let c0 = chars.next()?;
        let numbered = c0.is_ascii_digit()
            || (matches!(c0, '+' | '-' | '−' | '~' | '$' | '€' | '£') && chars.next().is_some_and(|c| c.is_ascii_digit()));
        if numbered {
            return None;
        }
        let (title, detail) = title_detail(text);
        if title.is_empty() {
            return None;
        }
        items.push(Finding {
            id: format!("x{i}"),
            severity,
            severity_text: token.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            refs: refs(t),
        });
    }
    let signal = lead_words(l).iter().any(|w| REVIEW_WORDS.contains(&w.as_str())) || items.iter().any(|f| !f.refs.is_empty());
    if !signal {
        return None;
    }
    Some(Findings { id: "b0".into(), title: None, source: source(l), items, provenance: Provenance::reply_text() })
}

/// The severity a reply's own token stands for; `None` for any other word.
pub(super) fn severity_of(token: &str) -> Option<Severity> {
    Some(match token.to_ascii_lowercase().as_str() {
        "critical" | "blocker" | "p0" => Severity::Critical,
        "high" | "major" | "p1" => Severity::High,
        "medium" | "moderate" | "p2" => Severity::Medium,
        "low" | "minor" | "p3" => Severity::Low,
        "info" | "nit" => Severity::Info,
        _ => return None,
    })
}

/// `**High**`, `**High:**`, `**High**:`, `[High]`, `**[High]**` or `High:` at the start of `t`:
/// the severity, the token as written and the text after it.
fn severity_lead(t: &str) -> Option<(Severity, &str, &str)> {
    let (s, bold) = match t.strip_prefix("**") {
        Some(r) => (r, true),
        None => (t, false),
    };
    let (s, bracket) = match s.strip_prefix('[') {
        Some(r) => (r, true),
        None => (s, false),
    };
    let len = s.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let token = &s[..len];
    let severity = severity_of(token)?;
    let mut rest = &s[len..];
    let mut marked = false;
    if bracket {
        rest = rest.strip_prefix(']')?;
        marked = true;
    }
    if bold {
        let colon = rest.strip_prefix(':');
        rest = colon.unwrap_or(rest).strip_prefix("**")?;
        marked = true;
    }
    if let Some(r) = rest.strip_prefix(':') {
        rest = r;
        marked = true;
    }
    if !marked || !rest.starts_with([' ', '\u{a0}']) && !rest.is_empty() {
        return None;
    }
    let text = rest.trim_start_matches([' ', '-', '–', '—', ':']).trim();
    (!text.is_empty()).then_some((severity, token, text))
}

/// A finding's title and detail: the bold lead, or the text before the first " — ", " – ",
/// " - ", ": " or ". "; the whole text when there is none.
fn title_detail(text: &str) -> (&str, &str) {
    if let Some(r) = text.strip_prefix("**") {
        if let Some(close) = r.find("**") {
            let detail = r[close + 2..].trim_start_matches([' ', '-', '–', '—', ':', '.']).trim();
            return (r[..close].trim(), detail);
        }
    }
    let at = [" — ", " – ", " - ", ": ", ". "].iter().filter_map(|sep| text.find(sep).map(|i| (i, sep.len()))).min();
    match at {
        Some((i, n)) if !text[i + n..].trim().is_empty() => (text[..i].trim(), text[i + n..].trim()),
        _ => (text, ""),
    }
}

/// `path/to/file.ext:123` in code spans: a path with a file extension, never a URL, a port or a
/// Windows path.
fn refs(t: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    for (k, span) in t.split('`').enumerate() {
        if k % 2 == 0 || span.contains(char::is_whitespace) || span.contains("://") || span.contains('\\') {
            continue;
        }
        let Some((path, line)) = span.rsplit_once(':') else { continue };
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !(line[digits..].is_empty() || line[digits..].starts_with('-')) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit_once('.').map(|(stem, ext)| (stem, ext));
        let good_ext = ext.is_some_and(|(stem, ext)| {
            !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) && ext.bytes().any(|b| b.is_ascii_alphabetic())
        });
        let path_chars = path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'));
        if !good_ext || !path_chars || path.to_ascii_lowercase().starts_with("localhost") {
            continue;
        }
        if out.len() < 10 {
            out.push(Ref { path: path.to_string(), line: line[..digits].parse().ok() });
        }
    }
    out
}

/// 3–15 items that each start with a time-like label and ":", "–" or "—"; the label is `when`.
/// A task box in front of every item is left out of the text.
pub(super) fn timeline(l: &List) -> Option<Steps> {
    if !(3..=15).contains(&l.items.len()) || !plain_items(l) {
        return None;
    }
    let all_tasks = l.items.iter().all(|t| task(t).is_some());
    let mut items = Vec::new();
    for t in &l.items {
        let body = if all_tasks { task(t)?.1 } else { t };
        let x = labeled(body)?;
        if !when_like(x.label) {
            return None;
        }
        items.push((x.rest.to_string(), None, Some(x.label.to_string())));
    }
    Some(steps_block(l, Variant::Timeline, items))
}

/// A task list of 3–20 items (`- [ ]` / `- [x]`, numbered or not); `done` comes from the box.
pub(super) fn checklist(l: &List) -> Option<Steps> {
    if !(3..=20).contains(&l.items.len()) || !plain_items(l) {
        return None;
    }
    let items = l
        .items
        .iter()
        .map(|t| task(t).map(|(done, text)| (text.to_string(), Some(done), None)))
        .collect::<Option<Vec<_>>>()?;
    Some(steps_block(l, Variant::Checklist, items))
}

/// A task item: whether it is checked, and its text. A box with no text after it is a plain item.
fn task(t: &str) -> Option<(bool, &str)> {
    let done = match t.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    let text = t[3..].strip_prefix(' ')?.trim();
    (!text.is_empty()).then_some((done, text))
}

/// A steps block of any variant from `(text, done, when)` per item; `owner` only from a literal
/// "Owner: X" or "(owner: X)" in the text.
pub(super) fn steps_block(l: &List, variant: Variant, items: Vec<(String, Option<bool>, Option<String>)>) -> Steps {
    let items = items
        .into_iter()
        .enumerate()
        .map(|(i, (text, done, when))| {
            let owner = owner_of(&text).map(str::to_string);
            Step { id: format!("s{i}"), text, done, when, owner }
        })
        .collect();
    Steps { id: "b0".into(), title: None, source: source(l), variant, items, provenance: Provenance::reply_text() }
}

/// The name in "(owner: X)", "(Owner: X)", or "Owner: X" / "owner: X" running to the end of the
/// item; at most 40 characters and 4 words.
pub(super) fn owner_of(text: &str) -> Option<&str> {
    for open in ["(owner: ", "(Owner: "] {
        if let Some(i) = text.find(open) {
            let r = &text[i + open.len()..];
            let end = r.find(')')?;
            return name(&r[..end]);
        }
    }
    for open in ["Owner: ", "owner: "] {
        if let Some(i) = text.find(open) {
            let before = text[..i].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric()) {
                continue;
            }
            let r = text[i + open.len()..].trim_end().trim_end_matches('.');
            return name(r);
        }
    }
    None
}

fn name(s: &str) -> Option<&str> {
    let s = s.trim();
    let ok = !s.is_empty()
        && s.chars().count() <= 40
        && s.split_whitespace().count() <= 4
        && !has_html(s)
        && !s.contains(['*', '[', ']', '`', ':', '(', ')']);
    ok.then_some(s)
}
