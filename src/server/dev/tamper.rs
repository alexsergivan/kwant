//! The test-tamper check (plan D6, design §5 part 4): changes that make a test easier to pass
//! instead of making the code right. Rule-based and deterministic over the diff text; no model.
//!
//! Five kinds, each found only in test code (test files, and the `#[cfg(test)]` part of a Rust
//! file):
//! - `deleted_assert`: a file loses more checks than it gains, and the lost ones aren't moved
//!   elsewhere in the diff. Not flagged when the diff removes more code than it adds (a feature
//!   and its tests going away together).
//! - `skip`: a new `#[ignore]`, `.skip`/`.only`, `xit`/`fit`, `{ skip: true }`, `pytest.mark.skip`,
//!   `unittest.skip`, `t.Skip` or `@Disabled`. A line that was only moved or re-indented isn't new.
//! - `loosened`: an exact check replaced by a weaker one on the same value (`toBe` → `toBeTruthy`,
//!   `assert_eq!` → `assert!(x.contains(..))`, `assertEqual` → `assertTrue`), or a tolerance made
//!   wider (`toBeCloseTo(x, 5)` → `toBeCloseTo(x, 2)`, `epsilon = 1e-6` → `1e-2`).
//! - `snapshot`: an existing snapshot's expected output rewritten (`__snapshots__`, `.snap`,
//!   `.ambr`, inline snapshots). New snapshots, added entries and insta's metadata lines
//!   (`assertion_line:` moving with the test) don't count.
//! - `coverage_drop`: in one area (a folder; a top-level test is matched to code by name), at
//!   least [`DROP_MIN`] test lines removed net while at least [`DROP_MIN`] code lines were added.
//!
//! The diff reader here ([`parse_files`]) is shared with `risk.rs`.

use super::facts::FileDiff;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    DeletedAssert,
    Skip,
    Loosened,
    Snapshot,
    CoverageDrop,
}

/// One tamper flag, as the review card shows it (plan §3.4 `tamper.flags[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub kind: Kind,
    pub file: String,
    /// Line in the PR's version of the file (for a removed line: where it was).
    pub line: u32,
    /// Up to [`EXCERPT_LINES`] diff lines, each starting with `-` or `+`.
    pub excerpt: String,
    pub why: String,
}

/// Net test lines removed, and code lines added, in one area before it's a coverage drop.
pub const DROP_MIN: u64 = 8;
const EXCERPT_LINES: usize = 4;
const EXCERPT_CHARS: usize = 160;

// ---------------------------------------------------------------------------------------------
// Reading a diff

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Del,
    Ctx,
}

#[derive(Debug, Clone)]
pub struct Line<'a> {
    pub op: Op,
    pub text: &'a str,
    /// Line number in the PR's version (for a removed line: the line it sat before).
    pub new: u32,
    /// Test code: a test file, or the test part of a Rust file.
    pub test: bool,
}

#[derive(Debug, Clone)]
pub struct Hunk<'a> {
    pub header: &'a str,
    pub lines: Vec<Line<'a>>,
}

/// One changed file with its hunks read.
#[derive(Debug, Clone)]
pub struct Parsed<'a> {
    pub path: &'a str,
    pub status: &'a str,
    pub additions: u64,
    pub deletions: u64,
    pub hunks: Vec<Hunk<'a>>,
    pub test_file: bool,
    pub snapshot: bool,
}

impl Parsed<'_> {
    pub fn lines(&self) -> impl Iterator<Item = &Line<'_>> {
        self.hunks.iter().flat_map(|h| h.lines.iter())
    }
    pub fn lang(&self) -> Lang {
        lang_of(self.path)
    }
}

pub fn parse_files(files: &[FileDiff]) -> Vec<Parsed<'_>> {
    files
        .iter()
        .map(|f| {
            let mut hunks = hunks(f.patch.as_deref().unwrap_or(""));
            let test_file = is_test_path(&f.path);
            mark_tests(&f.path, test_file, &mut hunks);
            Parsed {
                path: &f.path,
                status: &f.status,
                additions: f.additions,
                deletions: f.deletions,
                hunks,
                test_file,
                snapshot: is_snapshot_path(&f.path),
            }
        })
        .collect()
}

/// The hunks of a unified diff patch (GitHub's `patch` field, or `git diff` output).
pub fn hunks(patch: &str) -> Vec<Hunk<'_>> {
    let mut out: Vec<Hunk> = Vec::new();
    let mut new = 0u32;
    for raw in patch.lines() {
        if let Some(rest) = raw.strip_prefix("@@") {
            let (ranges, header) = rest.split_once("@@").unwrap_or((rest, ""));
            new = ranges
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            out.push(Hunk { header: header.trim(), lines: Vec::new() });
            continue;
        }
        let Some(h) = out.last_mut() else { continue };
        let (op, text) = match raw.as_bytes().first() {
            Some(b'+') => (Op::Add, &raw[1..]),
            Some(b'-') => (Op::Del, &raw[1..]),
            Some(b' ') => (Op::Ctx, &raw[1..]),
            Some(b'\\') => continue,
            _ => (Op::Ctx, raw),
        };
        h.lines.push(Line { op, text, new, test: false });
        if op != Op::Del {
            new += 1;
        }
    }
    out
}

fn mark_tests(path: &str, test_file: bool, hunks: &mut [Hunk]) {
    let rust = path.ends_with(".rs");
    for h in hunks {
        let mut in_test = test_file || (rust && (h.header.contains("mod tests") || h.header.contains("#[cfg(test)]")));
        for l in &mut h.lines {
            let t = l.text.trim_start();
            if rust && (t.starts_with("#[cfg(test)]") || t.starts_with("mod tests") || t.starts_with("#[test]") || t.starts_with("#[tokio::test")) {
                in_test = true;
            }
            l.test = in_test;
        }
    }
}

const TEST_DIRS: [&str; 6] = ["tests", "test", "__tests__", "spec", "specs", "e2e"];

/// A file that holds tests (by its path).
pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let (dirs, name) = p.rsplit_once('/').unwrap_or(("", &p));
    if dirs.split('/').any(|d| TEST_DIRS.contains(&d)) {
        return true;
    }
    name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("_test.rs")
        || name.ends_with("_tests.rs")
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name == "conftest.py"
        || name.ends_with("test.java")
        || name.ends_with("test.kt")
        || name.ends_with("tests.swift")
        || name.ends_with("_spec.rb")
        || name.ends_with("_test.rb")
}

/// A saved snapshot of expected output.
pub fn is_snapshot_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("__snapshots__/") || p.ends_with(".snap") || p.ends_with(".ambr") || p.contains(".approved.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Js,
    Py,
    Go,
    Other,
}

pub fn lang_of(path: &str) -> Lang {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "rs" => Lang::Rust,
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte" => Lang::Js,
        "py" => Lang::Py,
        "go" => Lang::Go,
        _ => Lang::Other,
    }
}

fn is_comment(t: &str, lang: Lang) -> bool {
    let t = t.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with("* ") || t == "*" || (lang == Lang::Py && t.starts_with('#'))
}

/// Whitespace removed: a line that was only re-indented or re-wrapped compares equal.
fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn clip(s: &str) -> String {
    let s = s.trim_end();
    if s.chars().count() <= EXCERPT_CHARS {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(EXCERPT_CHARS).collect::<String>())
    }
}

fn excerpt<'a>(lines: impl IntoIterator<Item = &'a Line<'a>>) -> String {
    lines
        .into_iter()
        .filter(|l| l.op != Op::Ctx)
        .take(EXCERPT_LINES)
        .map(|l| format!("{}{}", if l.op == Op::Add { '+' } else { '-' }, clip(l.text)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Byte offset of `word` in `s` with no identifier character right before it.
fn find_word(s: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find(word) {
        let at = from + i;
        if !s[..at].chars().next_back().is_some_and(ident_char) {
            return Some(at);
        }
        from = at + word.len();
    }
    None
}

/// The text inside the parenthesis that opens at `open` (or to the end of the line when it
/// doesn't close there), and the rest after it.
fn paren_args(s: &str, open: usize) -> (&str, &str) {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut prev = '\0';
    for (i, c) in s[open..].char_indices() {
        let at = open + i;
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (&s[open + 1..at], &s[at + 1..]);
                    }
                }
                _ => {}
            },
        }
        prev = if prev == '\\' && c == '\\' { '\0' } else { c };
    }
    (&s[(open + 1).min(s.len())..], "")
}

/// The top-level comma-separated arguments of `args`.
fn split_args(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let mut quote: Option<char> = None;
    for (i, c) in args.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(args[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    let last = args[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Assertions and how strict they are

/// How much a check pins down: the exact value, part of it, or only that there is something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    Existence = 1,
    Partial = 2,
    Exact = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub strength: Strength,
    /// The value it checks (first argument, whitespace removed); empty when it's on later lines.
    pub subject: String,
}

/// The check on `line`, if it has one.
pub fn assertion(line: &str, lang: Lang) -> Option<Assertion> {
    if is_comment(line, lang) {
        return None;
    }
    match lang {
        Lang::Rust => rust_assertion(line),
        Lang::Js => js_assertion(line),
        Lang::Py => py_assertion(line),
        Lang::Go => go_assertion(line),
        Lang::Other => None,
    }
}

fn mk(strength: Strength, subject: &str) -> Option<Assertion> {
    Some(Assertion { strength, subject: norm(subject) })
}

fn is_bool_lit(s: &str) -> bool {
    matches!(s.trim(), "true" | "false")
}

fn rust_assertion(line: &str) -> Option<Assertion> {
    for mac in ["assert_eq!(", "assert_ne!(", "assert_matches!(", "assert!("] {
        let Some(at) = find_word(line, mac) else { continue };
        let (args, _) = paren_args(line, at + mac.len() - 1);
        let parts = split_args(args);
        let first = parts.first().copied().unwrap_or("");
        return match mac {
            // `assert_eq!(x, true)` says no more than `assert!(x)` (clippy asks for the latter)
            "assert_eq!(" if parts.iter().any(|p| is_bool_lit(p)) => mk(Strength::Existence, first),
            "assert_eq!(" | "assert_matches!(" => mk(Strength::Exact, first),
            "assert_ne!(" => mk(Strength::Partial, first),
            _ => {
                let a = first.trim_start_matches(['!', '&', '*']);
                if let Some((l, _)) = a.split_once("==") {
                    mk(Strength::Exact, l)
                } else if [".contains(", "starts_with(", "ends_with(", "matches!(", ".any(", ".all(", "!=", ">", "<"].iter().any(|p| a.contains(p)) {
                    mk(Strength::Partial, a)
                } else {
                    mk(Strength::Existence, a)
                }
            }
        };
    }
    None
}

/// The matcher after `expect(..)`: the jest/vitest table, and chai's `.to.` chains.
fn js_matcher(rest: &str) -> Strength {
    let r = rest.trim_start();
    let not = r.starts_with(".not.");
    let m = r.trim_start_matches(".not").trim_start_matches('.');
    let name: String = m.chars().take_while(|c| ident_char(*c) || *c == '.').collect();
    let name = name.trim_start_matches("to.").trim_start_matches("be.").trim_start_matches("deep.");
    if not {
        return if ["toBeNull", "toBeUndefined", "toBeFalsy"].iter().any(|n| name.starts_with(n)) || m.starts_with("toBe(null") || m.starts_with("toBe(undefined") {
            Strength::Existence
        } else {
            Strength::Partial
        };
    }
    const EXACT: [&str; 12] =
        ["toBe(", "toEqual(", "toStrictEqual(", "toHaveLength(", "toBeNull(", "toBeUndefined(", "toHaveBeenCalledTimes(", "toHaveBeenCalledWith(", "toThrowError(", "equal(", "eql(", "equals("];
    const EXISTENCE: [&str; 8] = ["toBeTruthy(", "toBeFalsy(", "toBeDefined(", "toHaveBeenCalled(", "ok", "exist", "true", "false"];
    let call = &m[..m.find('(').map_or(m.len(), |i| i + 1)];
    let call = call.trim_start_matches("to.").trim_start_matches("be.").trim_start_matches("deep.");
    if EXACT.contains(&call) {
        Strength::Exact
    } else if EXISTENCE.iter().any(|e| call == *e || name == e.trim_end_matches('(')) {
        Strength::Existence
    } else {
        Strength::Partial
    }
}

fn js_assertion(line: &str) -> Option<Assertion> {
    if let Some(at) = find_word(line, "expect(") {
        let (args, rest) = paren_args(line, at + "expect".len());
        let first = split_args(args).first().copied().unwrap_or("").to_string();
        return mk(js_matcher(rest), &first);
    }
    if let Some(at) = find_word(line, "assert.") {
        let after = &line[at + "assert.".len()..];
        let name: String = after.chars().take_while(|c| ident_char(*c)).collect();
        let open = at + "assert.".len() + name.len();
        if !line[open..].starts_with('(') {
            return None;
        }
        let (args, _) = paren_args(line, open);
        let first = split_args(args).first().copied().unwrap_or("").to_string();
        let s = match name.as_str() {
            "equal" | "strictEqual" | "deepEqual" | "deepStrictEqual" => Strength::Exact,
            "ok" => Strength::Existence,
            _ => Strength::Partial,
        };
        return mk(s, &first);
    }
    if let Some(at) = find_word(line, "assert(") {
        let (args, _) = paren_args(line, at + "assert".len());
        let first = split_args(args).first().copied().unwrap_or("").to_string();
        return mk(if first.contains("===") || first.contains("==") { Strength::Exact } else { Strength::Existence }, &first);
    }
    None
}

fn py_assertion(line: &str) -> Option<Assertion> {
    let t = line.trim_start();
    // unittest style: `self.assertEqual(` and friends, an uppercase letter right after `assert`
    let unit = find_word(t, "self.assert").or_else(|| find_word(t, "assert")).filter(|&at| {
        let rest = &t[at..];
        let rest = rest.strip_prefix("self.").unwrap_or(rest);
        rest.as_bytes().get(6).is_some_and(|b| b.is_ascii_uppercase())
    });
    if let Some(at) = unit {
        let after = &t[at..];
        let name: String = after.chars().take_while(|c| ident_char(*c) || *c == '.').collect();
        let open = at + name.len();
        let (args, _) = if t[open..].starts_with('(') { paren_args(t, open) } else { ("", "") };
        let first = split_args(args).first().copied().unwrap_or("").to_string();
        let name = name.trim_start_matches("self.");
        let s = match name {
            "assertEqual" | "assertEquals" | "assertDictEqual" | "assertListEqual" | "assertSequenceEqual" | "assertTupleEqual" | "assertIs" | "assertIsNone" | "assertCountEqual" | "assertMultiLineEqual" => Strength::Exact,
            "assertTrue" | "assertFalse" | "assertIsNotNone" => Strength::Existence,
            _ => Strength::Partial,
        };
        return mk(s, &first);
    }
    let expr = t.strip_prefix("assert ")?;
    let expr = expr.split(", ").next().unwrap_or(expr).trim();
    if let Some((l, _)) = expr.split_once(" == ") {
        return mk(Strength::Exact, l);
    }
    if expr.ends_with(" is None") {
        return mk(Strength::Exact, expr.trim_end_matches(" is None"));
    }
    if let Some((_, r)) = expr.split_once(" not in ").or_else(|| expr.split_once(" in ")) {
        return mk(Strength::Partial, r);
    }
    if [" > ", " < ", " >= ", " <= ", " != ", "approx("].iter().any(|p| expr.contains(p)) {
        let l = expr.split([' ', '(']).next().unwrap_or(expr);
        return mk(Strength::Partial, l);
    }
    mk(Strength::Existence, expr.trim_start_matches("not ").trim_end_matches(" is not None"))
}

fn go_assertion(line: &str) -> Option<Assertion> {
    for pkg in ["assert.", "require."] {
        let Some(at) = find_word(line, pkg) else { continue };
        let after = &line[at + pkg.len()..];
        let name: String = after.chars().take_while(|c| ident_char(*c)).collect();
        let open = at + pkg.len() + name.len();
        if !line[open..].starts_with('(') {
            return None;
        }
        let (args, _) = paren_args(line, open);
        let parts = split_args(args);
        let subject = parts.get(if name.contains("Equal") { 2 } else { 1 }).copied().unwrap_or("");
        let s = match name.as_str() {
            "Equal" | "EqualValues" | "Exactly" | "JSONEq" | "Nil" | "Len" => Strength::Exact,
            "True" | "False" | "NotNil" | "NotEmpty" | "NoError" | "Error" => Strength::Existence,
            _ => Strength::Partial,
        };
        return mk(s, subject);
    }
    None
}

// ---------------------------------------------------------------------------------------------
// The check

/// Every tamper flag in `files`, at most one per kind per file, in file order.
pub fn check(files: &[FileDiff]) -> Vec<Flag> {
    let parsed = parse_files(files);
    let mut out = Vec::new();
    let (code_add, code_del) = code_lines(&parsed);
    let removing_code = code_del > code_add;
    // Checks the diff adds anywhere: a removed check that comes back elsewhere was moved
    let added_checks: Vec<String> = parsed
        .iter()
        .flat_map(|f| f.lines().filter(|l| l.op == Op::Add && assertion(l.text, f.lang()).is_some()).map(|l| norm(l.text)))
        .collect();
    for f in &parsed {
        if f.snapshot {
            out.extend(snapshot_file(f));
            continue;
        }
        if !removing_code {
            out.extend(deleted_asserts(f, &added_checks));
        }
        out.extend(skips(f));
        out.extend(loosened(f));
        out.extend(inline_snapshot(f));
    }
    out.extend(coverage_drop(&parsed));
    out
}

/// Code lines (not test code, not snapshots, code files only) added and removed in the diff.
fn code_lines(files: &[Parsed]) -> (u64, u64) {
    let (mut add, mut del) = (0, 0);
    for f in files.iter().filter(|f| is_code_path(f.path) && !f.snapshot) {
        for l in f.lines().filter(|l| !l.test) {
            match l.op {
                Op::Add => add += 1,
                Op::Del => del += 1,
                Op::Ctx => {}
            }
        }
    }
    (add, del)
}

/// A source file whose changes want tests (not docs, styles, data or build config).
pub fn is_code_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    if name.ends_with(".d.ts") || name.contains(".config.") || name == "build.rs" {
        return false;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(
        ext,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte" | "py" | "go" | "rb" | "java" | "kt" | "swift" | "c" | "cc" | "cpp" | "h" | "hpp" | "cs" | "php" | "scala" | "ex" | "exs"
    )
}

fn deleted_asserts(f: &Parsed, added_anywhere: &[String]) -> Option<Flag> {
    let lang = f.lang();
    let removed: Vec<&Line> = f.lines().filter(|l| l.op == Op::Del && l.test && assertion(l.text, lang).is_some()).collect();
    if removed.is_empty() {
        return None;
    }
    let mut pool: Vec<String> = added_anywhere.to_vec();
    let mut gone = Vec::new();
    for l in &removed {
        let n = norm(l.text);
        match pool.iter().position(|a| *a == n) {
            Some(i) => {
                pool.swap_remove(i);
            }
            None => gone.push(*l),
        }
    }
    let removed_texts: Vec<String> = removed.iter().map(|l| norm(l.text)).collect();
    let added = f.lines().filter(|l| l.op == Op::Add && l.test && assertion(l.text, lang).is_some() && !removed_texts.contains(&norm(l.text))).count();
    if gone.len() <= added {
        return None;
    }
    let first = gone[0];
    let n = gone.len() - added;
    Some(Flag {
        kind: Kind::DeletedAssert,
        file: f.path.to_string(),
        line: first.new,
        excerpt: excerpt(gone.iter().copied()),
        why: if n == 1 {
            "A check was removed from this test and nothing replaces it, so the test now proves less.".into()
        } else {
            format!("{n} checks were removed from these tests and nothing replaces them, so the tests now prove less.")
        },
    })
}

/// What `line` adds that stops a test from running, if anything.
fn skip_marker(line: &str, lang: Lang, test_file: bool) -> Option<&'static str> {
    if is_comment(line, lang) {
        return None;
    }
    let t = line.trim();
    match lang {
        Lang::Rust => {
            if t.starts_with("#[ignore") || (t.starts_with("#[cfg_attr(") && find_word(t, "ignore").is_some()) {
                return Some("#[ignore]");
            }
        }
        Lang::Js if test_file => {
            for name in ["it", "test", "describe", "context", "suite", "bench"] {
                for (m, label) in [(".skip(", ".skip"), (".skip.", ".skip"), (".skipIf(", ".skipIf"), (".only(", ".only"), (".only.", ".only")] {
                    if find_word(t, &format!("{name}{m}")).is_some() {
                        return Some(label);
                    }
                }
            }
            for x in ["xit(", "xtest(", "xdescribe(", "xcontext(", "fit(", "fdescribe(", "ftest("] {
                if find_word(t, x).is_some() {
                    return Some(if x.starts_with('x') { "xit" } else { "fit (.only)" });
                }
            }
            if find_word(t, "t.skip(").is_some() || find_word(t, "t.todo(").is_some() {
                return Some("t.skip");
            }
            // node:test's options object: `test('name', { skip: true }, ...)`
            for (opt, label) in [("skip:", "{ skip }"), ("only:", "{ only }")] {
                if let Some(i) = find_word(t, opt) {
                    let v = t[i + opt.len()..].trim_start();
                    if !v.starts_with("false") && ["test(", "it(", "describe(", "suite("].iter().any(|c| find_word(t, c).is_some()) {
                        return Some(label);
                    }
                }
            }
        }
        Lang::Py if test_file => {
            for (p, label) in [
                ("@pytest.mark.skip", "pytest.mark.skip"),
                ("@pytest.mark.xfail", "pytest.mark.xfail"),
                ("@unittest.skip", "unittest.skip"),
                ("pytest.skip(", "pytest.skip"),
                (".skipTest(", "skipTest"),
            ] {
                if t.contains(p) {
                    return Some(label);
                }
            }
        }
        Lang::Go if test_file && ["t.Skip(", "t.Skipf(", "t.SkipNow("].iter().any(|p| t.contains(p)) => return Some("t.Skip"),
        Lang::Other if test_file && (t.starts_with("@Disabled") || t.starts_with("@Ignore")) => return Some("@Disabled"),
        _ => {}
    }
    None
}

fn skips(f: &Parsed) -> Option<Flag> {
    let lang = f.lang();
    let mut removed: Vec<String> = f.lines().filter(|l| l.op == Op::Del && skip_marker(l.text, lang, f.test_file || l.test).is_some()).map(|l| norm(l.text)).collect();
    let mut found: Vec<(&Line, &'static str)> = Vec::new();
    for l in f.lines().filter(|l| l.op == Op::Add && (l.test || lang == Lang::Rust)) {
        let Some(m) = skip_marker(l.text, lang, f.test_file || l.test) else { continue };
        // Only moved or re-indented
        if let Some(i) = removed.iter().position(|r| *r == norm(l.text)) {
            removed.swap_remove(i);
            continue;
        }
        found.push((l, m));
    }
    let (first, marker) = *found.first()?;
    let focus = marker.contains("only");
    let why = match (focus, found.len()) {
        (true, _) => format!("`{marker}` runs this test alone, so every other test in the file is skipped."),
        (false, 1) => format!("This test no longer runs (`{marker}`), so it can't fail."),
        (false, n) => format!("{n} tests no longer run (`{marker}`), so they can't fail."),
    };
    Some(Flag { kind: Kind::Skip, file: f.path.to_string(), line: first.new, excerpt: excerpt([first]), why })
}

/// The runs of removed lines followed by added lines in each hunk: where a line was replaced.
fn change_blocks<'a>(f: &'a Parsed<'a>) -> Vec<(Vec<&'a Line<'a>>, Vec<&'a Line<'a>>)> {
    let mut out = Vec::new();
    for h in &f.hunks {
        let mut i = 0;
        while i < h.lines.len() {
            if h.lines[i].op != Op::Del {
                i += 1;
                continue;
            }
            let mut dels = Vec::new();
            while i < h.lines.len() && h.lines[i].op == Op::Del {
                dels.push(&h.lines[i]);
                i += 1;
            }
            let mut adds = Vec::new();
            while i < h.lines.len() && h.lines[i].op == Op::Add {
                adds.push(&h.lines[i]);
                i += 1;
            }
            if !adds.is_empty() {
                out.push((dels, adds));
            }
        }
    }
    out
}

/// `subject` appears in `line` as a whole expression (not as part of a longer name).
fn mentions(line: &str, subject: &str) -> bool {
    if subject.is_empty() {
        return false;
    }
    let n = norm(line);
    let mut from = 0;
    while let Some(i) = n[from..].find(subject) {
        let at = from + i;
        let before = n[..at].chars().next_back();
        let after = n[at + subject.len()..].chars().next();
        if !before.is_some_and(ident_char) && !after.is_some_and(ident_char) {
            return true;
        }
        from = at + subject.len().max(1);
    }
    false
}

fn loosened(f: &Parsed) -> Option<Flag> {
    let lang = f.lang();
    for (dels, adds) in change_blocks(f) {
        for d in dels.iter().filter(|l| l.test) {
            if let Some(old) = assertion(d.text, lang) {
                // A check on the same value as strict as before, or stricter, means it was kept
                let same: Vec<(&&Line, Assertion)> =
                    adds.iter().filter(|a| a.test).filter_map(|a| assertion(a.text, lang).map(|x| (a, x))).filter(|(a, x)| x.subject == old.subject || mentions(a.text, &old.subject)).collect();
                if !same.is_empty() && same.iter().all(|(_, x)| x.strength < old.strength) {
                    let (a, new) = &same[0];
                    let what = if new.strength == Strength::Existence { "that there is something" } else { "part of it" };
                    return Some(Flag {
                        kind: Kind::Loosened,
                        file: f.path.to_string(),
                        line: a.new,
                        excerpt: excerpt([*d, **a]),
                        why: format!("This check used to compare the exact value; now it only checks {what}, so a wrong result can still pass."),
                    });
                }
            }
            for a in adds.iter().filter(|l| l.test) {
                if let Some((o, n)) = widened(d.text, a.text) {
                    return Some(Flag {
                        kind: Kind::Loosened,
                        file: f.path.to_string(),
                        line: a.new,
                        excerpt: excerpt([*d, *a]),
                        why: format!("The allowed margin got wider ({o} → {n}), so a less exact result still passes."),
                    });
                }
            }
        }
    }
    None
}

const TOLERANCE_WORDS: [&str; 14] =
    ["tol", "tolerance", "epsilon", "eps", "delta", "approx", "tobecloseto", "abs_diff", "rel=", "abs=", "atol", "rtol", "within", "margin"];

/// The numbers in `s` with their byte ranges (not digits inside a name like `v2` or `polish063`).
fn numbers(s: &str) -> Vec<(usize, usize, f64)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_digit() || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit));
        let after_ident = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if !starts || after_ident {
            i += 1;
            continue;
        }
        let st = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b'_') {
            i += 1;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') && b.get(i + 1).is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+') {
            i += 2;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        let text: String = s[st..i].chars().filter(|c| *c != '_').collect();
        if let Ok(v) = text.trim_end_matches('.').parse::<f64>() {
            out.push((st, i, v));
        }
    }
    out
}

/// `old` → `new` only widened a tolerance: the same line apart from numbers, a tolerance word on
/// it, and every changed number looser. Returns the first changed pair as written.
fn widened(old: &str, new: &str) -> Option<(String, String)> {
    let lower = new.to_ascii_lowercase();
    if !TOLERANCE_WORDS.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let (no, nn) = (numbers(old), numbers(new));
    if no.len() != nn.len() || no.is_empty() {
        return None;
    }
    let skeleton = |s: &str, ns: &[(usize, usize, f64)]| {
        let mut out = String::new();
        let mut last = 0;
        for (a, b, _) in ns {
            out.push_str(&s[last..*a]);
            out.push('#');
            last = *b;
        }
        out.push_str(&s[last..]);
        norm(&out)
    };
    if skeleton(old, &no) != skeleton(new, &nn) {
        return None;
    }
    // toBeCloseTo(x, digits): fewer digits is looser; everywhere else a bigger number is
    let close_to = lower.contains("tobecloseto");
    let mut first = None;
    for (i, (o, n)) in no.iter().zip(&nn).enumerate() {
        if o.2 == n.2 {
            continue;
        }
        let looser = if close_to && i == no.len() - 1 { n.2 < o.2 } else { n.2 > o.2 };
        if !looser {
            return None;
        }
        first.get_or_insert((old[o.0..o.1].to_string(), new[n.0..n.1].to_string()));
    }
    first
}

/// insta's header lines, which change whenever the test moves.
fn snapshot_meta(t: &str) -> bool {
    let t = t.trim();
    t.is_empty()
        || t == "---"
        || ["source:", "expression:", "assertion_line:", "input_file:", "info:", "description:", "// Jest Snapshot", "// Vitest Snapshot", "// Snapshot v"].iter().any(|p| t.starts_with(p))
}

fn snapshot_file(f: &Parsed) -> Option<Flag> {
    if f.status == "added" || f.status == "removed" {
        return None;
    }
    let rewritten: Vec<&Line> = f.lines().filter(|l| l.op == Op::Del && !snapshot_meta(l.text)).collect();
    let first = rewritten.first()?;
    let added: Vec<&Line> = f.lines().filter(|l| l.op == Op::Add && !snapshot_meta(l.text)).take(2).collect();
    Some(Flag {
        kind: Kind::Snapshot,
        file: f.path.to_string(),
        line: first.new,
        excerpt: excerpt(rewritten.iter().take(2).copied().chain(added)),
        why: format!(
            "The saved expected output was rewritten ({} line{} changed). Check the new output is right, not just accepted.",
            rewritten.len(),
            if rewritten.len() == 1 { "" } else { "s" }
        ),
    })
}

/// A rewritten inline snapshot (`toMatchInlineSnapshot(`…`)`, insta's `@"…"`).
fn inline_snapshot(f: &Parsed) -> Option<Flag> {
    if !(f.test_file || f.lines().any(|l| l.test)) {
        return None;
    }
    for h in &f.hunks {
        let mut inside = false;
        let mut hit: Vec<&Line> = Vec::new();
        for l in &h.lines {
            let opens = l.text.contains("toMatchInlineSnapshot(") || l.text.contains("assert_snapshot!(") || l.text.contains("assert_debug_snapshot!(");
            if opens {
                inside = true;
            }
            if inside && l.test && l.op == Op::Del && (!opens || l.text.contains('`') || l.text.contains("@\"")) {
                hit.push(l);
            }
            if inside && !opens && (l.text.contains("`)") || l.text.trim_end().ends_with("\");") || l.text.trim() == ");") {
                inside = false;
            }
        }
        if let Some(first) = hit.first() {
            let adds: Vec<&Line> = h.lines.iter().filter(|l| l.op == Op::Add).take(2).collect();
            return Some(Flag {
                kind: Kind::Snapshot,
                file: f.path.to_string(),
                line: first.new,
                excerpt: excerpt(hit.iter().take(2).copied().chain(adds)),
                why: "The expected output written in this test was replaced. Check the new output is right, not just accepted.".into(),
            });
        }
    }
    None
}

/// Folder of `path` without its test folders: `frontend/tests/x.test.mjs` → `frontend`.
pub fn area_of(path: &str) -> String {
    let dirs = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    dirs.split('/').filter(|d| !d.is_empty() && !TEST_DIRS.contains(&d.to_ascii_lowercase().as_str())).collect::<Vec<_>>().join("/")
}

/// The name a file is about: `Sidebar.test.tsx` → `sidebar`, `reactions_replies_test.rs` →
/// `reactions_replies`, `mod.rs`/`index.ts` → their folder's name.
pub fn stem_of(path: &str) -> String {
    let p = path.to_ascii_lowercase();
    let mut parts = p.rsplit('/');
    let name = parts.next().unwrap_or("");
    let base = name.split('.').next().unwrap_or(name);
    let base = if matches!(base, "mod" | "index" | "lib" | "main" | "__init__") { parts.next().unwrap_or(base) } else { base };
    let base = base.strip_prefix("test_").unwrap_or(base);
    base.trim_end_matches("_tests").trim_end_matches("_test").trim_end_matches("_spec").to_string()
}

/// The test file `test` is about code in `area` (holding `code_paths`): its folder (test folders
/// left out) is the area or contains it, or its name contains a code file's name, or the reverse.
pub fn test_covers(test: &str, area: &str, code_paths: &[&str]) -> bool {
    let ta = area_of(test);
    if !ta.is_empty() && (ta == area || area.starts_with(&format!("{ta}/"))) {
        return true;
    }
    let ts = stem_of(test);
    code_paths.iter().any(|c| {
        let cs = stem_of(c);
        cs.len() >= 3 && ts.len() >= 3 && (ts.contains(&cs) || cs.contains(&ts))
    })
}

fn coverage_drop(files: &[Parsed]) -> Option<Flag> {
    // Code lines added per area
    let mut areas: HashMap<String, (u64, Vec<&str>)> = HashMap::new();
    for f in files.iter().filter(|f| is_code_path(f.path) && !f.test_file && !f.snapshot) {
        let add = f.lines().filter(|l| l.op == Op::Add && !l.test).count() as u64;
        if add > 0 {
            let e = areas.entry(area_of(f.path)).or_default();
            e.0 += add;
            e.1.push(f.path);
        }
    }
    let mut keys: Vec<&String> = areas.keys().collect();
    keys.sort();
    for area in keys {
        let (code_add, paths) = &areas[area];
        if *code_add < DROP_MIN {
            continue;
        }
        let (mut del, mut add) = (0u64, 0u64);
        let mut worst: Option<(&Parsed, u64)> = None;
        for f in files.iter().filter(|f| !f.snapshot) {
            let own = !f.test_file && area_of(f.path) == *area;
            if !(own || (f.test_file && test_covers(f.path, area, paths))) {
                continue;
            }
            let d = f.lines().filter(|l| l.op == Op::Del && l.test).count() as u64;
            let a = f.lines().filter(|l| l.op == Op::Add && l.test).count() as u64;
            del += d;
            add += a;
            if d > 0 && worst.is_none_or(|(_, w)| d > w) {
                worst = Some((f, d));
            }
        }
        let net = del.saturating_sub(add);
        if net < DROP_MIN {
            continue;
        }
        let (f, _) = worst?;
        let gone: Vec<&Line> = f.lines().filter(|l| l.op == Op::Del && l.test).collect();
        let shown = if area.is_empty() { "the project root".to_string() } else { format!("`{area}`") };
        return Some(Flag {
            kind: Kind::CoverageDrop,
            file: f.path.to_string(),
            line: gone[0].new,
            excerpt: excerpt(gone.iter().take(2).copied()),
            why: format!("{net} test lines were removed in {shown} while {code_add} lines of code were added there."),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, status: &str, patch: &str) -> FileDiff {
        let additions = patch.lines().filter(|l| l.starts_with('+')).count() as u64;
        let deletions = patch.lines().filter(|l| l.starts_with('-')).count() as u64;
        FileDiff { path: path.into(), status: status.into(), additions, deletions, patch: Some(patch.into()), previous_path: None }
    }
    fn modified(path: &str, patch: &str) -> FileDiff {
        file(path, "modified", patch)
    }
    fn kinds(files: &[FileDiff]) -> Vec<Kind> {
        check(files).into_iter().map(|f| f.kind).collect()
    }
    /// `n` lines of new code in `path`.
    fn code(path: &str, n: usize) -> FileDiff {
        let body: String = (0..n).map(|i| format!("+    let v{i} = step({i});\n")).collect();
        file(path, "modified", &format!("@@ -10,2 +10,{} @@ fn run() {{\n     let a = 1;\n{body}     a\n", n + 2))
    }

    // --- the diff reader ---

    #[test]
    fn hunks_number_lines_on_the_new_side() {
        let h = hunks("@@ -10,3 +10,4 @@ fn x() {\n a\n-b\n+c\n+d\n e\n\\ No newline at end of file\n");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].header, "fn x() {");
        let ops: Vec<(Op, &str, u32)> = h[0].lines.iter().map(|l| (l.op, l.text, l.new)).collect();
        assert_eq!(ops, vec![(Op::Ctx, "a", 10), (Op::Del, "b", 11), (Op::Add, "c", 11), (Op::Add, "d", 12), (Op::Ctx, "e", 13)]);
    }

    #[test]
    fn rust_test_module_lines_are_test_code_and_the_rest_is_not() {
        let f = [modified(
            "src/db/reactions.rs",
            "@@ -60,2 +60,3 @@ impl Database {\n     let a = 1;\n+    let b = 2;\n@@ -488,2 +489,4 @@ mod tests {\n     }\n+    #[tokio::test]\n+    async fn t() {}\n",
        )];
        let p = parse_files(&f);
        assert!(!p[0].hunks[0].lines.iter().any(|l| l.test));
        assert!(p[0].hunks[1].lines.iter().all(|l| l.test));
    }

    #[test]
    fn test_paths_are_known_by_folder_and_name() {
        for p in ["tests/reactions_replies_test.rs", "frontend/tests/ia.test.mjs", "src/a/__tests__/X.tsx", "app/test_views.py", "pkg/x_test.go", "web/Card.spec.ts"] {
            assert!(is_test_path(p), "{p}");
        }
        for p in ["src/server/reactions.rs", "frontend/src/components/Sidebar.tsx", "src/testing_utils.rs", "docs/tests.md.bak/x"] {
            assert!(!is_test_path(p) || p.starts_with("docs/tests"), "{p}");
        }
        assert!(!is_test_path("src/contest.rs"));
    }

    #[test]
    fn assertions_are_ranked_by_how_much_they_pin_down() {
        let r = |s: &str| assertion(s, Lang::Rust).map(|a| (a.strength, a.subject));
        assert_eq!(r("    assert_eq!(out, \"Bearer ••••\");"), Some((Strength::Exact, "out".into())));
        assert_eq!(r("    assert!(out.contains(\"Bearer\"));"), Some((Strength::Partial, "out.contains(\"Bearer\")".into())));
        assert_eq!(r("    assert!(x.is_some());"), Some((Strength::Existence, "x.is_some()".into())));
        assert_eq!(r("    assert!(a == b);"), Some((Strength::Exact, "a".into())));
        assert_eq!(r("    assert_eq!(done, true);").map(|x| x.0), Some(Strength::Existence));
        assert_eq!(r("    debug_assert!(x);"), None);
        assert_eq!(r("    // assert_eq!(a, b);"), None);
        let j = |s: &str| assertion(s, Lang::Js).map(|a| a.strength);
        assert_eq!(j("expect(sum(1, 2)).toBe(3);"), Some(Strength::Exact));
        assert_eq!(j("expect(sum(1, 2)).toBeTruthy();"), Some(Strength::Existence));
        assert_eq!(j("expect(list).toContain('a');"), Some(Strength::Partial));
        assert_eq!(j("expect(x).not.toBeNull();"), Some(Strength::Existence));
        assert_eq!(j("assert.equal(actions.length, lines.length);"), Some(Strength::Exact));
        assert_eq!(j("assert.ok(lines.length >= 3);"), Some(Strength::Existence));
        assert_eq!(j("assert.match(cls, /bottom-2/);"), Some(Strength::Partial));
        let p = |s: &str| assertion(s, Lang::Py).map(|a| a.strength);
        assert_eq!(p("    self.assertEqual(total, 3)"), Some(Strength::Exact));
        assert_eq!(p("    self.assertTrue(total)"), Some(Strength::Existence));
        assert_eq!(p("    assert total == 3"), Some(Strength::Exact));
        assert_eq!(p("    assert \"Bearer\" in out"), Some(Strength::Partial));
        assert_eq!(p("    assert out"), Some(Strength::Existence));
        assert_eq!(p("    assertion_count = 3"), None);
    }

    // --- deleted_assert ---

    #[test]
    fn a_removed_check_with_nothing_in_its_place_is_flagged() {
        let f = [
            code("src/server/activity.rs", 12),
            modified(
                "src/server/activity.rs",
                "@@ -410,6 +410,5 @@ mod tests {\n     #[test]\n     fn redacts_bearer_header() {\n         let out = redact(\"Authorization: Bearer abc\");\n-        assert_eq!(out, \"Authorization: Bearer ••••\");\n         assert!(!out.is_empty());\n     }\n",
            ),
        ];
        let flags = check(&f);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert_eq!(flags[0].kind, Kind::DeletedAssert);
        assert_eq!(flags[0].line, 413);
        assert!(flags[0].excerpt.starts_with("-        assert_eq!(out"), "{}", flags[0].excerpt);
    }

    #[test]
    fn checks_rewritten_or_moved_are_not_deleted() {
        // #290's real test edit: one check replaced by five stricter ones
        let f = [
            code("frontend/src/components/Sidebar.tsx", 10),
            modified(
                "frontend/tests/polish063.test.mjs",
                "@@ -150,3 +150,5 @@ test('3', () => {\n   const phone = render();\n-  for (const [, cls] of phone.matchAll(/x/g)) assert.doesNotMatch(cls, /pr-9/);\n+  assert.doesNotMatch(phone, /pr-9/);\n+  assert.doesNotMatch(phone, /data-bot-row-actions/);\n });\n",
            ),
        ];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
        // A check moved to another test file
        let moved = [
            modified("tests/a_test.rs", "@@ -5,3 +5,2 @@ async fn one() {\n     let x = run();\n-    assert_eq!(x.len(), 2);\n }\n"),
            modified("tests/b_test.rs", "@@ -5,2 +5,3 @@ async fn two() {\n     let x = run();\n+    assert_eq!(x.len(), 2);\n }\n"),
        ];
        assert!(kinds(&moved).is_empty());
    }

    #[test]
    fn a_feature_removed_with_its_tests_is_not_tampering() {
        let removed_code: String = (0..30).map(|i| format!("-    old_step({i});\n")).collect();
        let f = [
            modified("src/server/legacy.rs", &format!("@@ -1,31 +1,1 @@\n fn keep() {{}}\n{removed_code}")),
            file("tests/legacy_test.rs", "removed", "@@ -1,4 +0,0 @@\n-#[test]\n-fn old() {\n-    assert_eq!(old_step(1), 2);\n-}\n"),
        ];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
    }

    // --- skip ---

    #[test]
    fn new_skips_and_focus_are_flagged_in_every_language() {
        let cases = [
            ("src/server/activity.rs", "@@ -420,2 +420,3 @@ mod tests {\n     #[test]\n+    #[ignore]\n     fn redacts_quoted_value() {\n", "#[ignore]"),
            ("frontend/tests/card.test.tsx", "@@ -3,2 +3,2 @@ describe('card', () => {\n-  it('shows the title', () => {\n+  it.skip('shows the title', () => {\n", ".skip"),
            ("frontend/tests/card.test.tsx", "@@ -3,2 +3,2 @@ describe('card', () => {\n-  it('shows the title', () => {\n+  it.only('shows the title', () => {\n", ".only"),
            ("web/x.spec.js", "@@ -3,2 +3,2 @@\n-  it('works', () => {\n+  xit('works', () => {\n", "xit"),
            ("frontend/tests/ia.test.mjs", "@@ -3,1 +3,1 @@\n-test('QA a', () => {\n+test('QA a', { skip: 'flaky' }, () => {\n", "{ skip }"),
            ("app/tests/test_views.py", "@@ -3,1 +3,2 @@\n+@pytest.mark.skip(reason=\"later\")\n def test_view():\n", "pytest.mark.skip"),
            ("pkg/x_test.go", "@@ -3,1 +3,2 @@ func TestX(t *testing.T) {\n+\tt.Skip(\"later\")\n", "t.Skip"),
        ];
        for (path, patch, marker) in cases {
            let flags = check(&[modified(path, patch)]);
            assert_eq!(flags.len(), 1, "{path}: {flags:?}");
            assert_eq!(flags[0].kind, Kind::Skip, "{path}");
            assert!(flags[0].why.contains(marker), "{path}: {}", flags[0].why);
        }
    }

    #[test]
    fn skip_lookalikes_are_not_skips() {
        // Iterator .skip in Rust code, a moved #[ignore], a comment, `skip: false`, .skip outside tests
        let f = [
            modified("src/server/reactions.rs", "@@ -1,1 +1,2 @@\n+    let rest = items.iter().skip(1).collect::<Vec<_>>();\n"),
            modified("src/db/x.rs", "@@ -400,4 +400,4 @@ mod tests {\n-  #[ignore]\n+    #[ignore]\n     fn slow() {}\n"),
            modified("frontend/tests/a.test.mjs", "@@ -1,1 +1,3 @@\n+// it.skip('old', ...) was removed\n+test('x', { skip: false }, () => {});\n+const rows = list.skip(2);\n"),
            modified("frontend/src/pager.ts", "@@ -1,1 +1,2 @@\n+export const page = { skip: true, it: 1 };\n"),
        ];
        assert!(kinds(&f).iter().all(|k| *k != Kind::Skip), "{:?}", check(&f));
    }

    // --- loosened ---

    #[test]
    fn an_exact_check_made_weaker_is_loosened() {
        let cases = [
            ("src/server/activity.rs", "@@ -416,3 +416,3 @@ mod tests {\n         let out = redact(line);\n-        assert_eq!(out, \"Authorization: Bearer ••••\");\n+        assert!(out.contains(\"Bearer\"));\n"),
            ("frontend/tests/sum.test.ts", "@@ -4,1 +4,1 @@ test('sum', () => {\n-  expect(sum(1, 2)).toBe(3);\n+  expect(sum(1, 2)).toBeTruthy();\n"),
            ("frontend/tests/rows.test.mjs", "@@ -4,1 +4,1 @@ test('rows', () => {\n-  assert.equal(actions.length, lines.length);\n+  assert.ok(actions.length);\n"),
            ("app/tests/test_total.py", "@@ -4,1 +4,1 @@ def test_total():\n-    self.assertEqual(total(cart), 30)\n+    self.assertTrue(total(cart))\n"),
        ];
        for (path, patch) in cases {
            let flags = check(&[modified(path, patch)]);
            assert_eq!(flags.iter().map(|f| f.kind).collect::<Vec<_>>(), vec![Kind::Loosened], "{path}: {flags:?}");
        }
        let f = check(&[modified(cases[0].0, cases[0].1)]);
        assert_eq!(f[0].line, 417);
        assert_eq!(f[0].excerpt, "-        assert_eq!(out, \"Authorization: Bearer ••••\");\n+        assert!(out.contains(\"Bearer\"));");
    }

    #[test]
    fn a_wider_tolerance_is_loosened() {
        let cases = [
            ("frontend/tests/geo.test.ts", "@@ -4,1 +4,1 @@\n-  expect(dist(a, b)).toBeCloseTo(12.5, 5);\n+  expect(dist(a, b)).toBeCloseTo(12.5, 1);\n", "5 → 1"),
            ("src/geo.rs", "@@ -90,1 +90,1 @@ mod tests {\n-        assert!((d - 12.5).abs() < 1e-6, \"tolerance\");\n+        assert!((d - 12.5).abs() < 0.5, \"tolerance\");\n", "1e-6 → 0.5"),
            ("tests/test_geo.py", "@@ -4,1 +4,1 @@\n-    assert dist(a, b) == pytest.approx(12.5, rel=1e-6)\n+    assert dist(a, b) == pytest.approx(12.5, rel=1e-2)\n", "1e-6 → 1e-2"),
        ];
        for (path, patch, change) in cases {
            let flags = check(&[modified(path, patch)]);
            assert_eq!(flags.len(), 1, "{path}: {flags:?}");
            assert_eq!(flags[0].kind, Kind::Loosened);
            assert!(flags[0].why.contains(change), "{path}: {}", flags[0].why);
        }
    }

    #[test]
    fn ordinary_check_edits_are_not_loosened() {
        let f = [
            // A new expected value, equally strict (#303-style)
            modified("tests/a_test.rs", "@@ -4,1 +4,1 @@ async fn t() {\n-    assert_eq!(rows.len(), 1);\n+    assert_eq!(rows.len(), 2);\n"),
            // Stricter: toBeTruthy → toBe(true)
            modified("frontend/tests/b.test.ts", "@@ -4,1 +4,1 @@\n-  expect(ok).toBeTruthy();\n+  expect(ok).toBe(true);\n"),
            // #301's real edit: a match turned into a doesNotMatch plus a new check
            modified(
                "frontend/tests/ia.test.mjs",
                "@@ -251,1 +251,2 @@\n-      assert.match(cls, /overflow-wrap:anywhere/, `${name}: a name with no spaces still breaks`);\n+      assert.doesNotMatch(cls, /overflow-wrap:anywhere|break-all|break-words/, `${name}: wraps between words`);\n+      assert.match(cls, /\\btext-ellipsis\\b/, `${name}: ends in …`);\n",
            ),
            // clippy's bool_assert_comparison fix
            modified("src/x.rs", "@@ -50,1 +50,1 @@ mod tests {\n-        assert_eq!(done, true);\n+        assert!(done);\n"),
            // A tolerance made tighter, and a timeout made longer
            modified("frontend/tests/geo.test.ts", "@@ -4,2 +4,2 @@\n-  expect(d).toBeCloseTo(12.5, 2);\n-  await waitFor(done, { timeout: 1000 });\n+  expect(d).toBeCloseTo(12.5, 4);\n+  await waitFor(done, { timeout: 5000 });\n"),
            // Split into two checks, one as strict as before
            modified("tests/c_test.rs", "@@ -4,1 +4,2 @@ async fn t() {\n-    assert_eq!(out, \"a b\");\n+    assert!(out.starts_with(\"a\"));\n+    assert_eq!(out, \"a b\".to_string());\n"),
        ];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
    }

    // --- snapshot ---

    #[test]
    fn a_rewritten_snapshot_is_flagged() {
        let f = [modified(
            "frontend/src/components/__snapshots__/Card.test.tsx.snap",
            "@@ -1,6 +1,6 @@\n // Jest Snapshot v1\n exports[`Card shows the total 1`] = `\n <div>\n-  Total: 30 €\n+  Total: NaN €\n </div>\n`;\n",
        )];
        let flags = check(&f);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].kind, Kind::Snapshot);
        assert!(flags[0].excerpt.contains("-  Total: 30 €") && flags[0].excerpt.contains("+  Total: NaN €"), "{}", flags[0].excerpt);
        let inline = [modified("frontend/tests/card.test.tsx", "@@ -8,4 +8,4 @@\n   expect(render()).toMatchInlineSnapshot(`\n-    \"Total: 30 €\"\n+    \"Total: NaN €\"\n   `);\n")];
        assert_eq!(kinds(&inline), vec![Kind::Snapshot]);
    }

    #[test]
    fn new_snapshots_and_moved_tests_are_not_rewrites() {
        let f = [
            file("tests/snapshots/card__total.snap", "added", "@@ -0,0 +1,5 @@\n+---\n+source: tests/card.rs\n+expression: total\n+---\n+30 €\n"),
            // insta's line number moves with the test
            modified("tests/snapshots/card__title.snap", "@@ -1,5 +1,5 @@\n ---\n source: tests/card.rs\n-assertion_line: 41\n+assertion_line: 44\n expression: title\n"),
            // A new entry added to an existing jest snapshot
            modified("src/__snapshots__/Row.test.tsx.snap", "@@ -9,0 +10,3 @@\n+exports[`Row wraps 1`] = `\n+<div>Researcher</div>\n+`;\n"),
            // An obsolete snapshot file deleted
            file("src/__snapshots__/Old.test.tsx.snap", "removed", "@@ -1,2 +0,0 @@\n-exports[`Old 1`] = `x`;\n-\n"),
        ];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
    }

    // --- coverage_drop ---

    #[test]
    fn tests_removed_where_code_was_added_is_a_coverage_drop() {
        let gone: String = (0..12).map(|i| format!("-    run_case({i});\n")).collect();
        let f = [
            code("src/server/billing/invoice.rs", 20),
            modified("src/server/billing/tests/invoice_test.rs", &format!("@@ -1,13 +1,1 @@\n fn cases() {{\n{gone}")),
        ];
        let flags = check(&f);
        assert_eq!(flags.len(), 1, "{flags:?}");
        assert_eq!(flags[0].kind, Kind::CoverageDrop);
        assert_eq!(flags[0].file, "src/server/billing/tests/invoice_test.rs");
        assert!(flags[0].why.starts_with("12 test lines were removed in `src/server/billing` while 20 lines"), "{}", flags[0].why);
        // A top-level test matched to the code by name
        let f = [code("src/server/reactions.rs", 20), modified("tests/reactions_replies_test.rs", &format!("@@ -1,13 +1,1 @@\n fn cases() {{\n{gone}"))];
        assert_eq!(kinds(&f), vec![Kind::CoverageDrop]);
    }

    #[test]
    fn test_rewrites_and_unrelated_test_cleanups_are_not_a_coverage_drop() {
        let gone: String = (0..12).map(|i| format!("-    run_case({i});\n")).collect();
        let back: String = (0..10).map(|i| format!("+    run_case_v2({i});\n")).collect();
        // Rewritten tests: −12 +10 is a net 2
        let f = [code("src/server/billing/invoice.rs", 20), modified("src/server/billing/tests/invoice_test.rs", &format!("@@ -1,13 +1,11 @@\n fn cases() {{\n{gone}{back}"))];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
        // Tests removed in a different area from the new code
        let f = [code("frontend/src/components/Sidebar.tsx", 20), modified("tests/reactions_replies_test.rs", &format!("@@ -1,13 +1,1 @@\n fn cases() {{\n{gone}"))];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
        // Only a little code added
        let f = [code("src/server/reactions.rs", 3), modified("tests/reactions_replies_test.rs", &format!("@@ -1,13 +1,1 @@\n fn cases() {{\n{gone}"))];
        assert!(kinds(&f).is_empty(), "{:?}", check(&f));
    }

    #[test]
    fn the_three_real_merges_have_no_tamper() {
        for n in [290, 301, 303] {
            let facts: super::super::facts::DevFacts = serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr{n}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
            assert!(check(&facts.files).is_empty(), "#{n}: {:?}", check(&facts.files));
        }
    }
}
