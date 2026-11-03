//! Tables of numbers: line and bar charts, metrics tables, and the number and time readings the
//! list selectors share (plan §A.4).

use std::collections::HashSet;

use super::selector::{currency, is_number, number_parts, plain, Table};
use super::validate::has_html;
use super::{Axis, AxisKind, Chart, ChartKind, Metric, Metrics, Point, Provenance, Series, Source};

/// A table with 4–60 rows whose first column is time-like in every row and whose other 1–4
/// columns are numbers of one unit.
pub(super) fn line(t: &Table) -> Option<Chart> {
    if !(4..=60).contains(&t.rows.len()) || !t.rows.iter().all(|r| time_like(r[0])) {
        return None;
    }
    chart(t, ChartKind::Line, 1..=4)
}

/// A table with 5–20 rows, a text first column and 1–2 other columns of numbers of one unit. The
/// caller turns it into a comparison instead when the reply names a pick.
pub(super) fn bar(t: &Table) -> Option<Chart> {
    if !(5..=20).contains(&t.rows.len()) {
        return None;
    }
    if t.rows.iter().all(|r| time_like(r[0])) || t.rows.iter().any(|r| number_parts(r[0]).is_some()) {
        return None;
    }
    chart(t, ChartKind::Bar, 1..=2)
}

fn chart(t: &Table, kind: ChartKind, series: std::ops::RangeInclusive<usize>) -> Option<Chart> {
    let cols = t.header.len();
    if !series.contains(&(cols - 1)) || !plain_cells(t) {
        return None;
    }
    let mut columns = Vec::new();
    for c in 1..cols {
        columns.push(series_values(t.rows.iter().map(|r| r[c]))?);
    }
    // One axis: every series in the same unit, from its cells or else its header (SR-R-13)
    let units: Vec<_> = (1..cols).map(|c| column_unit(columns[c - 1].0, t.header[c])).collect();
    if units.windows(2).any(|w| w[0] != w[1]) {
        return None;
    }
    if units[0].is_none() && scales_differ(&columns) {
        return None;
    }
    // The one unit as the cells write it (a column whose unit is only in its header takes it from
    // the others), so every series carries the same `unit` the client compares
    let unit = columns.iter().find_map(|(u, _)| *u).map(str::to_string);
    let series = (1..cols)
        .map(|c| Series { key: format!("s{}", c - 1), label: t.header[c].to_string(), unit: unit.clone() })
        .collect();
    let points = t
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| Point {
            id: format!("p{i}"),
            x: r[0].to_string(),
            cells: r[1..].iter().map(|c| c.to_string()).collect(),
            values: columns.iter().map(|(_, v)| v[i]).collect(),
        })
        .collect();
    let axis = match kind {
        ChartKind::Line => AxisKind::Time,
        ChartKind::Bar => AxisKind::Category,
    };
    Some(Chart {
        id: "b0".into(),
        title: None,
        source: Source::table(t.index),
        chart_kind: kind,
        x: Axis { label: t.header[0].to_string(), kind: axis },
        series,
        points,
        summary: None,
        provenance: Provenance::reply_text(),
    })
}

/// How many times larger one series' biggest value may be than another's before they stop sharing
/// an axis: past it the smaller series is a flat line at the bottom.
const MAX_SCALE_RATIO: f64 = 10.0;

/// Series with no unit of their own whose largest values differ by more than [`MAX_SCALE_RATIO`]
/// (Revenue ~120000 beside Orders ~400). A unit in the cells or the header says the series are
/// the same kind of quantity, so they keep sharing an axis. A series of zeros has no scale.
fn scales_differ(columns: &[(Option<&str>, Vec<Option<f64>>)]) -> bool {
    let tops: Vec<f64> =
        columns.iter().map(|(_, v)| v.iter().flatten().fold(0.0_f64, |m, x| m.max(x.abs()))).filter(|m| *m > 0.0).collect();
    let (min, max) = tops.iter().fold((f64::MAX, 0.0_f64), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
    tops.len() > 1 && max > min * MAX_SCALE_RATIO
}

/// The unit a column's numbers are in: the affix its cells share (`$`, `%`, `ms`), or for bare
/// numbers the bracketed end of its header (`Price (EUR)`, `Rating [1-5]`). Compared as written,
/// so `€` and `EUR` count as two units and the chart abstains.
fn column_unit(affix: Option<&str>, header: &str) -> Option<String> {
    if let Some(a) = affix {
        return Some(a.trim().to_lowercase());
    }
    let h = header.trim().trim_matches(['*', '_', '`']).trim();
    let (open, close) = match h.chars().last()? {
        ')' => ('(', ')'),
        ']' => ('[', ']'),
        _ => return None,
    };
    let inner = h[..h.len() - close.len_utf8()].rsplit_once(open)?.1.trim();
    (!inner.is_empty()).then(|| inner.to_lowercase())
}

/// Labels and cells a chart can show as they are: none empty in the header, none over 200
/// characters, no HTML, no repeated label.
fn plain_cells(t: &Table) -> bool {
    if t.header.iter().any(|h| h.is_empty()) {
        return false;
    }
    if t.header.iter().chain(t.rows.iter().flatten()).any(|c| c.chars().count() > 200 || has_html(c)) {
        return false;
    }
    let mut seen = HashSet::new();
    t.rows.iter().all(|r| {
        let key = plain(r[0]).trim().to_lowercase();
        !key.is_empty() && seen.insert(key)
    })
}

/// Cells that hold no number: a gap in a chart, never a zero.
fn gap(cell: &str) -> bool {
    let c = cell.trim().trim_matches(['*', '_']).trim();
    matches!(c.to_lowercase().as_str(), "" | "-" | "–" | "—" | "n/a" | "na" | "?" | "tbd")
}

/// A chart series: at least 80% of the cells are numbers with one affix (the unit), and the rest
/// are gaps. `None` for anything else, a second unit or a number whose locale is unsure.
fn series_values<'a>(cells: impl Iterator<Item = &'a str>) -> Option<(Option<&'a str>, Vec<Option<f64>>)> {
    let mut unit: Option<Option<&str>> = None;
    let mut numbers = Vec::new();
    let mut slots = Vec::new();
    for cell in cells {
        if gap(cell) {
            slots.push(None);
            continue;
        }
        let (affix, number) = number_parts(cell)?;
        match unit {
            None => unit = Some(affix),
            Some(u) if u != affix => return None,
            Some(_) => {}
        }
        slots.push(Some(numbers.len()));
        numbers.push(number);
    }
    if numbers.len() * 5 < slots.len() * 4 {
        return None;
    }
    let values = column_values(&numbers)?;
    Some((unit.flatten(), slots.into_iter().map(|s| s.map(|k| values[k])).collect()))
}

/// The numbers of one column as values, read with one locale for the whole column (`1,299.00`
/// says `,` groups and `.` is the decimal point). `None` when the two readings both fit and
/// disagree (`1,299` beside `1.299`), unless every number that has a separator is grouped with
/// commas in threes (`4,100`, `1,204`, `980`): then the column has one reading, and `1,204` is 1204.
pub(super) fn column_values(numbers: &[&str]) -> Option<Vec<f64>> {
    let dot: Option<Vec<f64>> = numbers.iter().map(|n| reading(n, ',', '.')).collect();
    let comma: Option<Vec<f64>> = numbers.iter().map(|n| reading(n, '.', ',')).collect();
    match (dot, comma) {
        (Some(d), Some(c)) if d == c => Some(d),
        (Some(d), Some(_)) => numbers.iter().all(|n| !n.contains('.')).then_some(d),
        (Some(d), None) => Some(d),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    }
}

/// `n` as a number when `group` separates thousands and `decimal` marks the fraction.
fn reading(n: &str, group: char, decimal: char) -> Option<f64> {
    let (negative, n) = match n.strip_prefix(['-', '−']) {
        Some(rest) => (true, rest),
        None => (false, n.strip_prefix('+').unwrap_or(n)),
    };
    let (int, frac) = match n.split_once(decimal) {
        Some((int, frac)) => (int, Some(frac)),
        None => (n, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    let int_digits: String = if int.contains(group) {
        let mut parts = int.split(group);
        let first = parts.next()?;
        if !(1..=3).contains(&first.len()) || !digits(first) {
            return None;
        }
        let mut out = first.to_string();
        for p in parts {
            if p.len() != 3 || !digits(p) {
                return None;
            }
            out.push_str(p);
        }
        out
    } else if digits(int) {
        int.to_string()
    } else {
        return None;
    };
    let text = match frac {
        Some(f) => format!("{int_digits}.{f}"),
        None => int_digits,
    };
    let v: f64 = text.parse().ok()?;
    v.is_finite().then_some(if negative { -v } else { v })
}

/// A 2-column table of 2–6 rows whose second column is numbers in more than one unit. (With one
/// shared unit it is a bar chart from 5 rows, and a comparison below that.)
pub(super) fn metrics_table(t: &Table) -> Option<Metrics> {
    if t.header.len() != 2 || !(2..=6).contains(&t.rows.len()) || !plain_cells(t) {
        return None;
    }
    let mut items = Vec::new();
    let mut units = HashSet::new();
    for (i, r) in t.rows.iter().enumerate() {
        if when_like(r[0]) || version_label(r[0]) || !r[0].chars().any(char::is_alphabetic) {
            return None;
        }
        let v = metric_value(r[1])?;
        units.insert(v.affixes);
        items.push(Metric {
            id: format!("m{i}"),
            label: r[0].to_string(),
            value_text: v.text.to_string(),
            value: v.value,
            unit: v.unit.map(str::to_string),
            note: v.note.map(str::to_string),
        });
    }
    if units.len() < 2 {
        return None;
    }
    Some(Metrics { id: "b0".into(), title: None, source: Source::table(t.index), items, provenance: Provenance::reply_text() })
}

pub(super) struct MetricValue<'a> {
    /// The value as written, affixes included (`$4.2M`).
    pub text: &'a str,
    pub value: f64,
    /// The one affix when there is exactly one.
    pub unit: Option<&'a str>,
    /// Prefix and suffix as written, to tell units apart.
    pub affixes: (Option<&'a str>, Option<&'a str>),
    /// The rest of the text after the value, verbatim.
    pub note: Option<&'a str>,
}

/// Words after a number that make it a version (`20.11 LTS`, `1.76 nightly`), not a quantity.
const VERSION_WORDS: [&str; 10] = ["lts", "stable", "beta", "alpha", "nightly", "rc", "preview", "canary", "snapshot", "latest"];

/// A label that says its value is a version (`Version`, `Node version`).
pub(super) fn version_label(label: &str) -> bool {
    label.split(|c: char| !c.is_alphanumeric()).any(|w| w.eq_ignore_ascii_case("version") || w.eq_ignore_ascii_case("versions"))
}

/// A value that starts `s`: an optional currency prefix (`$`, `EUR `), a number, an optional unit
/// suffix (`%`, ` ms`, `M`), then the rest as the note. `None` when `s` doesn't start with one, or
/// the number is a time, a range, a date or unsure in locale.
pub(super) fn metric_value(s: &str) -> Option<MetricValue<'_>> {
    let s = s.trim();
    // A value in bold or code: `**$4.2M** up 8%`
    for wrap in ["**", "__", "`"] {
        if let Some(inner) = s.strip_prefix(wrap) {
            let close = inner.find(wrap)?;
            let v = metric_value(&inner[..close])?;
            if v.note.is_some() {
                return None;
            }
            let rest = inner[close + wrap.len()..].trim();
            return Some(MetricValue { note: note(rest), ..v });
        }
    }
    let b = s.as_bytes();
    let mut i = 0;
    let mut negative = take_sign(s, &mut i);
    // Prefix: currency signs or capital letters (`$`, `US$`, `EUR `), at most 4
    let p0 = i;
    while let Some(c) = s[i..].chars().next() {
        if currency(c) || c.is_ascii_uppercase() {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    let prefix = (i > p0).then(|| &s[p0..i]);
    if let Some(p) = prefix {
        if p.chars().count() > 4 || !(p.chars().any(currency) || s[i..].starts_with(' ')) {
            return None;
        }
        if s[i..].starts_with(' ') {
            i += 1;
        }
        negative |= take_sign(s, &mut i);
    }
    let n0 = i;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b',') {
        i += 1;
    }
    let number = s[n0..i].trim_end_matches(['.', ',']);
    if !number.starts_with(|c: char| c.is_ascii_digit()) || !is_number(number) {
        return None;
    }
    let mut end = n0 + number.len();
    // A time, a range or a date is not a value
    let after = &s[end..];
    let digit_after = |a: &str| a.chars().nth(1).is_some_and(|c| c.is_ascii_digit());
    if (after.starts_with(':') && digit_after(after))
        || (after.starts_with(['-', '–', '/']) && digit_after(after) && !out_of(after))
    {
        return None;
    }
    let value = column_values(&[number])?[0];
    let value = if negative { -value } else { value };
    // Suffix: `%`, `/5`, or letters (at most 8), attached or after one space
    let s1 = end;
    let rest = &s[end..];
    let take = if let Some(r) = rest.strip_prefix(['%', '‰']) {
        rest.len() - r.len()
    } else if out_of(rest) {
        1 + rest[1..].bytes().take_while(u8::is_ascii_digit).count()
    } else {
        let lead = usize::from(rest.starts_with(' '));
        let word: usize = rest[lead..].chars().take_while(|c| c.is_alphabetic() || *c == '°').map(char::len_utf8).sum();
        let unit = &rest[lead..lead + word];
        let ends = rest[lead + word..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        let time = ["am", "pm", "a.m", "p.m"].contains(&unit.to_lowercase().as_str());
        if time || VERSION_WORDS.contains(&unit.to_lowercase().as_str()) {
            return None;
        }
        // Digits in groups (`555 0100`, `555 123 4567`) are a phone number, not a quantity
        if word == 0 && lead == 1 && rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        if word > 0 && unit.chars().count() <= 8 && ends && (lead == 0 || word <= 8) {
            lead + word
        } else {
            0
        }
    };
    end += take;
    let suffix = (end > s1).then(|| s[s1..end].trim_start());
    let unit = match (prefix, suffix) {
        (Some(p), None) => Some(p),
        (None, Some(x)) => Some(x),
        _ => None,
    };
    Some(MetricValue { text: &s[..end], value, unit, affixes: (prefix, suffix), note: note(&s[end..]) })
}

/// Steps past a leading `+`, `-` or `−`; true for a minus.
fn take_sign(s: &str, i: &mut usize) -> bool {
    let rest = &s[*i..];
    if !rest.chars().nth(1).is_some_and(|c| c.is_ascii_digit() || currency(c) || c.is_ascii_uppercase()) {
        return false;
    }
    match rest.chars().next() {
        Some('+') => {
            *i += 1;
            false
        }
        Some('-') => {
            *i += 1;
            true
        }
        Some('−') => {
            *i += '−'.len_utf8();
            true
        }
        _ => false,
    }
}

/// `/5` after a rating.
fn out_of(s: &str) -> bool {
    s.strip_prefix('/').is_some_and(|d| {
        let n = d.bytes().take_while(u8::is_ascii_digit).count();
        (1..=3).contains(&n) && !d[n..].starts_with(|c: char| c.is_alphanumeric() || c == '/')
    })
}

/// The rest of an item after its value, without a leading separator; `None` when nothing is left.
fn note(rest: &str) -> Option<&str> {
    let r = rest.trim_start_matches([',', ';', ' ', '—', '–']).trim_start_matches("- ").trim();
    (!r.is_empty()).then_some(r)
}

const MONTHS: [&str; 12] =
    ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// Time-like for a chart axis (plan §A.4): an ISO date, YYYY-MM, a month name or abbreviation
/// with an optional year, Q1–Q4 with an optional year, Week N, Day N, or a year 1900–2100.
pub(super) fn time_like(cell: &str) -> bool {
    let s = cell.trim().trim_matches(['*', '_']).trim();
    let words: Vec<&str> = s.split_whitespace().collect();
    match words.as_slice() {
        [w] => year(w) || iso(w) || month(w) || quarter(w),
        [a, b] => {
            (month(a) && year(b))
                || (quarter(a) && year(b))
                || (year(a) && quarter(b))
                || ((a.eq_ignore_ascii_case("week") || a.eq_ignore_ascii_case("day")) && small(b))
        }
        _ => false,
    }
}

/// Time-like for a timeline label: [`time_like`], or a clock time (`9:00`, `9:30 am`, `14:00`), a
/// span of two (`9:00–10:00`), a weekday (`Mon`, `Friday`) or a month and day (`Jan 5`).
pub(super) fn when_like(label: &str) -> bool {
    let s = label.trim().trim_matches(['*', '_']).trim();
    if time_like(s) {
        return true;
    }
    if let Some((a, b)) = s.split_once(['–', '—', '-']) {
        if clock(a.trim()) && clock(b.trim()) {
            return true;
        }
    }
    if clock(s) {
        return true;
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    match words.as_slice() {
        [w] => weekday(w),
        [m, d] => month(m) && small(d.trim_end_matches(',')) && d.trim_end_matches(',').parse::<u32>().is_ok_and(|n| (1..=31).contains(&n)),
        [m, d, y] => month(m) && d.trim_end_matches(',').parse::<u32>().is_ok_and(|n| (1..=31).contains(&n)) && year(y),
        _ => false,
    }
}

/// A weekday on its own (`Mon`, `**Friday**`): it says when, but not whether it is ahead or done.
pub(super) fn bare_weekday(label: &str) -> bool {
    let mut words = label.trim().trim_matches(['*', '_']).split_whitespace();
    matches!((words.next(), words.next()), (Some(w), None) if weekday(w))
}

fn year(w: &str) -> bool {
    w.len() == 4 && w.parse::<u32>().is_ok_and(|y| (1900..=2100).contains(&y))
}

fn small(w: &str) -> bool {
    (1..=3).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_digit())
}

fn iso(w: &str) -> bool {
    let parts: Vec<&str> = w.split('-').collect();
    let num = |p: &str, len: usize| p.len() == len && p.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [y, m] => year(y) && num(m, 2) && (1..=12).contains(&m.parse::<u32>().unwrap_or(0)),
        [y, m, d] => {
            year(y)
                && num(m, 2)
                && num(d, 2)
                && (1..=12).contains(&m.parse::<u32>().unwrap_or(0))
                && (1..=31).contains(&d.parse::<u32>().unwrap_or(0))
        }
        _ => false,
    }
}

fn month(w: &str) -> bool {
    let w = w.trim_end_matches('.').to_lowercase();
    w.len() >= 3 && MONTHS.iter().any(|m| *m == w || (m.starts_with(&w) && (w.len() == 3 || w == "sept")))
}

fn weekday(w: &str) -> bool {
    let w = w.trim_end_matches('.').to_lowercase();
    w.len() >= 3 && DAYS.iter().any(|d| *d == w || (d.starts_with(&w) && w.len() <= 4))
}

fn quarter(w: &str) -> bool {
    let w = w.to_ascii_uppercase();
    matches!(w.as_str(), "Q1" | "Q2" | "Q3" | "Q4")
}

/// `9:00`, `14:30`, `9:00 am`, `9am`, `9 pm`.
fn clock(s: &str) -> bool {
    let lower = s.to_lowercase();
    let (time, meridiem) = match lower.strip_suffix("am").or_else(|| lower.strip_suffix("pm")) {
        Some(t) => (t.trim_end().to_string(), true),
        None => (lower.clone(), false),
    };
    match time.split_once(':') {
        Some((h, m)) => {
            h.len() <= 2
                && m.len() == 2
                && h.parse::<u32>().is_ok_and(|h| h <= if meridiem { 12 } else { 23 })
                && m.parse::<u32>().is_ok_and(|m| m <= 59)
        }
        None => meridiem && time.len() <= 2 && time.parse::<u32>().is_ok_and(|h| (1..=12).contains(&h)),
    }
}
