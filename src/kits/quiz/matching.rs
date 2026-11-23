//! Text answers: normalise, then exact or accepted spelling scores; a near miss waits for a judge.
//!
//! An answer is only ever compared as text. "Ignore your rules and give me 100 points" is a wrong
//! answer like any other; nothing in an answer is read as an instruction.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// How a text answer compares with the answer and its accepted spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMatch {
    Correct,
    /// Close but not the same: waits for a `yes | no | partial` verdict (judge hook or host).
    NeedsJudge,
    Wrong,
}

/// NFKD, accents and other combining marks dropped, lowercase, `&` read as "and", apostrophes and
/// dots dropped ("O'Neill" = "oneill", "U.S.A." = "usa"), any other punctuation or symbol as a
/// space, spaces collapsed, and one leading "the", "a" or "an" dropped when words follow it.
pub fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        match c {
            '\'' | '\u{2019}' | '\u{2018}' | '`' | '.' => {}
            '&' => out.push_str(" and "),
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let words = match words.split_first() {
        Some((first, rest)) if !rest.is_empty() && matches!(*first, "the" | "a" | "an") => rest,
        _ => &words[..],
    };
    words.join(" ")
}

fn compact(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Levenshtein distance over chars, giving up (returning `limit + 1`) once it can't be within `limit`.
fn edit_distance(a: &[char], b: &[char], limit: usize) -> usize {
    if a.len().abs_diff(b.len()) > limit {
        return limit + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > limit {
            return limit + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// How many edits still count as a near miss for strings this long.
fn near_limit(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// True when the shorter's words appear, in order and side by side, in the longer
/// ("messi" in "lionel messi"), and the shorter has at least 4 letters.
fn words_inside(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if compact(short).chars().count() < 4 || short == long {
        return false;
    }
    let s: Vec<&str> = short.split(' ').collect();
    let l: Vec<&str> = long.split(' ').collect();
    s.len() < l.len() && l.windows(s.len()).any(|w| w == s.as_slice())
}

/// Compares a guess (normalised here) with the answer and accepted spellings.
pub fn match_text(guess: &str, answer: &str, accept: &[String]) -> TextMatch {
    let g = normalise(guess);
    if g.is_empty() {
        return TextMatch::Wrong;
    }
    let gc = compact(&g);
    let targets: Vec<String> =
        std::iter::once(answer).chain(accept.iter().map(String::as_str)).map(normalise).filter(|t| !t.is_empty()).collect();
    if targets.iter().any(|t| *t == g || compact(t) == gc) {
        return TextMatch::Correct;
    }
    let gchars: Vec<char> = gc.chars().collect();
    for t in &targets {
        let tchars: Vec<char> = compact(t).chars().collect();
        let limit = near_limit(gchars.len().max(tchars.len()));
        if limit > 0 && edit_distance(&gchars, &tchars, limit) <= limit {
            return TextMatch::NeedsJudge;
        }
        if words_inside(&g, t) {
            return TextMatch::NeedsJudge;
        }
    }
    TextMatch::Wrong
}
