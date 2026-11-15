//! 17d: the server owns the list of task event kinds (src/db/event_kinds.rs).
//!
//! 1. A source scan of src/ and tests/ finds every kind an emitter writes to `task_events` and checks it
//!    is on `TASK_EVENT_KINDS`, and that every listed kind is still written somewhere.
//! 2. The checked-in frontend copy (frontend/src/generated/taskEventKinds.json) equals the list.
//!
//! The emitters: `insert_event` and `log_task_event` (the two writes, which also `debug_assert!` the
//! kind), their wrappers `Database::task_event`, autofix's `change_event` and the auditor's and the
//! verifier's `event()`, `TaskChange { kind }` (moved through `insert_event` by `move_task`), and the
//! literal `INSERT INTO task_events` statements.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kwnt::db::event_kinds::{is_task_event_kind, TASK_EVENT_KINDS};

const JSON_PATH: &str = "frontend/src/generated/taskEventKinds.json";
const REGENERATE: &str = "KWANT_WRITE_TASK_EVENT_KINDS=1 cargo test --test task_event_kinds_test";

/// Emitter calls: the function's name, the kind's argument index, and the one file it lives in
/// (`None`: any file).
const CALLS: &[(&str, usize, Option<&str>)] = &[
    ("insert_event", 3, None),
    ("log_task_event", 1, None),
    ("task_event", 2, None),
    ("change_event", 2, Some("src/db/autofix.rs")),
    ("event", 2, Some("src/server/missions/auditor.rs")),
    ("event", 2, Some("src/server/missions/verifier.rs")),
];

/// Calls whose kind is not a literal or a named const: (file, enclosing fn, the kind argument, the
/// kinds it can be). An empty list means the fn forwards its own `kind` parameter, so its callers
/// are what the scan checks. A kind built with `format!` lists each expansion here.
const RUNTIME: &[(&str, &str, &str, &[&str])] = &[
    ("src/db/checkpoints.rs", "insert_checkpoint", "kind", &["checkpoint", "checkpoint_failed"]),
    ("src/db/checkpoints.rs", "task_event", "kind", &[]),
    ("src/db/autofix.rs", "insert_attempt", "kind", &["autofix_finished", "autofix_scheduled"]),
    ("src/db/autofix.rs", "move_attempt", "kind", &["autofix_finished", "autofix_scheduled", "autofix_started"]),
    ("src/db/autofix.rs", "change_event", "kind", &[]),
    ("src/db/missions.rs", "move_task", "&change.kind", &[]),
    ("src/server/missions/auditor.rs", "event", "kind", &[]),
    ("src/server/missions/verifier.rs", "event", "kind", &[]),
    ("src/server/missions/lessons.rs", "log_attribution", "kind", &["lessons_held_out", "lessons_shown"]),
    ("src/server/missions/checkpoints.rs", "restore", "kind", &["held", "released"]),
];

/// Bound (`?`) kinds in a literal INSERT are allowed only in the two writes that assert them.
const BOUND_INSERTS: &[(&str, &str)] = &[("src/db/missions.rs", "log_task_event"), ("src/db/missions.rs", "insert_event")];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The file with `//` comment lines blanked (kept as empty lines so line numbers hold).
fn code(text: &str) -> String {
    text.lines().map(|l| if l.trim_start().starts_with("//") { "" } else { l }).collect::<Vec<_>>().join("\n")
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// From `open` (an opening bracket), the text up to its matching close, skipping string literals.
fn balanced(text: &str, open: usize) -> &str {
    let b = text.as_bytes();
    let (mut depth, mut i) = (0i32, open);
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unbalanced brackets from byte {open}");
}

/// Top-level comma-separated items, trimmed.
fn split_top(args: &str) -> Vec<String> {
    let b = args.as_bytes();
    let (mut out, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0, 0);
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(args[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = args[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

/// The name of the fn whose body holds byte `at`: the nearest `fn name` before it.
fn enclosing_fn(text: &str, at: usize) -> String {
    let mut best = String::new();
    let mut from = 0;
    while let Some(i) = text[from..at].find("fn ") {
        let i = from + i;
        let before_ok = i == 0 || !ident_char(text[..i].chars().last().unwrap());
        let name: String = text[i + 3..].chars().take_while(|c| ident_char(*c)).collect();
        if before_ok && !name.is_empty() && text[i + 3 + name.len()..].starts_with(['(', '<']) {
            best = name;
        }
        from = i + 3;
    }
    best
}

enum Arg {
    Literal(String),
    Const(String),
    Runtime(String),
}

fn classify(arg: &str) -> Arg {
    let a = arg.trim();
    let inner = a.strip_prefix('&').unwrap_or(a);
    let lit = |s: &str| (s.len() >= 2 && s.starts_with('"') && s.ends_with('"') && !s[1..s.len() - 1].contains('"')).then(|| s[1..s.len() - 1].to_string());
    for suffix in [".into()", ".to_string()", ".to_owned()", ""] {
        if let Some(s) = inner.strip_suffix(suffix).and_then(lit) {
            return Arg::Literal(s);
        }
    }
    if let Some(s) = inner.strip_prefix("String::from(").and_then(|s| s.strip_suffix(')')).and_then(lit) {
        return Arg::Literal(s);
    }
    let last = inner.rsplit("::").next().unwrap_or(inner);
    if !last.is_empty() && last.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && last.chars().any(|c| c.is_ascii_uppercase()) {
        return Arg::Const(last.to_string());
    }
    Arg::Runtime(a.to_string())
}

#[derive(Default)]
struct Scan {
    /// kind -> where it is written (file:line)
    written: BTreeMap<String, Vec<String>>,
    problems: Vec<String>,
    /// calls found per emitter, so a rename can't make the scan pass by finding nothing
    calls: BTreeMap<&'static str, usize>,
    runtime_used: BTreeSet<(String, String, String)>,
}

impl Scan {
    fn kind(&mut self, kind: &str, at: &str) {
        self.written.entry(kind.to_string()).or_default().push(at.to_string());
    }

    fn arg(&mut self, arg: &str, file: &str, text: &str, pos: usize, consts: &BTreeMap<String, BTreeSet<String>>) {
        let at = format!("{file}:{}", line_of(text, pos));
        match classify(arg) {
            Arg::Literal(k) => self.kind(&k, &at),
            Arg::Const(name) => match consts.get(&name).map(|v| v.iter().collect::<Vec<_>>()) {
                Some(v) if v.len() == 1 => self.kind(v[0], &format!("{at} ({name})")),
                Some(v) => self.problems.push(format!("{at}: const {name} has several values {v:?}; give it one name")),
                None => self.problems.push(format!("{at}: the kind is const {name}, which the scan can't find as `const {name}: &str = \"...\"`")),
            },
            Arg::Runtime(expr) => {
                let func = enclosing_fn(text, pos);
                match RUNTIME.iter().find(|(f, n, e, _)| *f == file && *n == func && *e == expr) {
                    Some((_, _, _, kinds)) => {
                        self.runtime_used.insert((file.to_string(), func.clone(), expr.clone()));
                        for k in *kinds {
                            if !text.contains(&format!("\"{k}\"")) {
                                self.problems.push(format!("{at}: RUNTIME says fn {func} writes {k:?}, but {file} has no \"{k}\" literal"));
                            }
                            self.kind(k, &format!("{at} (fn {func})"));
                        }
                    }
                    None => self.problems.push(format!(
                        "{at}: fn {func} writes a kind built at run time ({expr}). List each kind it can be in RUNTIME in tests/task_event_kinds_test.rs, \
                         and add any new one to TASK_EVENT_KINDS (src/db/event_kinds.rs)"
                    )),
                }
            }
        }
    }
}

fn consts(files: &[(String, String)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, text) in files {
        let mut from = 0;
        while let Some(i) = text[from..].find("const ") {
            let i = from + i;
            from = i + 6;
            let name: String = text[i + 6..].chars().take_while(|c| ident_char(*c)).collect();
            let rest = &text[i + 6 + name.len()..];
            for ty in [": &str = \"", ": &'static str = \""] {
                if let Some(v) = rest.strip_prefix(ty) {
                    if let Some(end) = v.find('"') {
                        out.entry(name.clone()).or_default().insert(v[..end].to_string());
                    }
                }
            }
        }
    }
    out
}

fn scan() -> Scan {
    let root = root();
    let mut paths = Vec::new();
    rust_files(&root.join("src"), &mut paths);
    rust_files(&root.join("tests"), &mut paths);
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| (p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), code(&std::fs::read_to_string(p).unwrap())))
        .collect();
    let consts = consts(&files);
    let mut s = Scan::default();
    for (file, text) in &files {
        if file == "tests/task_event_kinds_test.rs" {
            continue;
        }
        // 1. Emitter calls
        for (name, idx, only) in CALLS {
            if only.is_some_and(|f| f != file) {
                continue;
            }
            let pat = format!("{name}(");
            let mut from = 0;
            while let Some(i) = text[from..].find(&pat) {
                let i = from + i;
                from = i + pat.len();
                let prev = text[..i].chars().last();
                if prev.is_some_and(|c| ident_char(c) || (*name == "event" && (c == '.' || c == ':'))) || text[..i].ends_with("fn ") {
                    continue;
                }
                *s.calls.entry(name).or_default() += 1;
                let args = split_top(balanced(text, i + name.len()));
                match args.get(*idx) {
                    Some(a) => s.arg(a, file, text, i, &consts),
                    None => s.problems.push(format!("{file}:{}: {name}( has no argument {idx}", line_of(text, i))),
                }
            }
        }
        // 2. TaskChange { kind } (no kind: Default's "state")
        let mut from = 0;
        while let Some(i) = text[from..].find("TaskChange {") {
            let i = from + i;
            from = i + 12;
            let before = text[..i].trim_end();
            if ["struct", "for", "->", "impl"].iter().any(|w| before.ends_with(w)) {
                continue;
            }
            *s.calls.entry("TaskChange").or_default() += 1;
            let fields = split_top(balanced(text, i + 11));
            match fields.iter().find(|f| f.starts_with("kind:") || f.as_str() == "kind") {
                Some(f) => {
                    let v = f.strip_prefix("kind:").unwrap_or(f).trim().to_string();
                    s.arg(&v, file, text, i, &consts);
                }
                None if fields.iter().any(|f| f.starts_with("..")) => s.kind("state", &format!("{file}:{} (TaskChange default)", line_of(text, i))),
                None => s.problems.push(format!("{file}:{}: TaskChange without kind or ..default", line_of(text, i))),
            }
        }
        // 3. Literal INSERT INTO task_events
        let mut from = 0;
        while let Some(i) = text[from..].find("INSERT INTO task_events") {
            let i = from + i;
            from = i + 23;
            let at = format!("{file}:{}", line_of(text, i));
            *s.calls.entry("INSERT").or_default() += 1;
            let sql = text[i..].replace("\\\n", " ");
            let cols_at = sql.find('(').unwrap();
            let cols: Vec<String> = sql[cols_at + 1..cols_at + sql[cols_at..].find(')').unwrap()].split(',').map(|c| c.trim().to_string()).collect();
            let k = cols.iter().position(|c| c == "kind").unwrap_or_else(|| panic!("{at}: INSERT without a kind column"));
            let values_at = sql.find("VALUES (").unwrap_or_else(|| panic!("{at}: INSERT without VALUES (")) + 7;
            let b = sql.as_bytes();
            let (mut depth, mut j) = (0, values_at);
            let end = loop {
                match b[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break j;
                        }
                    }
                    _ => {}
                }
                j += 1;
            };
            let values = split_top(&sql[values_at + 1..end]);
            let v = values.get(k).map(String::as_str).unwrap_or("");
            if let Some(lit) = v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
                s.kind(lit, &at);
            } else if v == "?" {
                let func = enclosing_fn(text, i);
                if !BOUND_INSERTS.contains(&(file.as_str(), func.as_str())) {
                    s.problems.push(format!("{at}: fn {func} binds the kind of a literal INSERT INTO task_events; write it through insert_event or log_task_event"));
                }
            } else {
                s.problems.push(format!("{at}: can't read the kind {v:?} of a literal INSERT INTO task_events"));
            }
        }
    }
    s
}

#[test]
fn task_event_kinds_are_sorted_unique_and_snake_case() {
    for w in TASK_EVENT_KINDS.windows(2) {
        assert!(w[0] < w[1], "TASK_EVENT_KINDS must be sorted with no duplicates: {:?} then {:?}", w[0], w[1]);
    }
    for k in TASK_EVENT_KINDS {
        assert!(!k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{k:?} is not snake_case");
        assert!(is_task_event_kind(k));
    }
    assert!(!is_task_event_kind("brand_new_kind"));
}

#[test]
fn every_kind_an_emitter_writes_is_listed_and_every_listed_kind_is_written() {
    let s = scan();
    let mut problems = s.problems.clone();
    for (kind, at) in &s.written {
        if !is_task_event_kind(kind) {
            problems.push(format!(
                "{}: writes task event kind {kind:?}, which is not in TASK_EVENT_KINDS. Add it to src/db/event_kinds.rs (sorted), regenerate {JSON_PATH} with `{REGENERATE}`, and give it plain words in eventText (frontend/src/missions.ts)",
                at.join(", ")
            ));
        }
    }
    for k in TASK_EVENT_KINDS {
        if !s.written.contains_key(*k) {
            problems.push(format!("TASK_EVENT_KINDS lists {k:?}, but no emitter writes it; remove it (and regenerate {JSON_PATH})"));
        }
    }
    for (f, n, e, _) in RUNTIME {
        if !s.runtime_used.contains(&(f.to_string(), n.to_string(), e.to_string())) {
            problems.push(format!("RUNTIME lists {f} fn {n} ({e}), but the scan no longer finds that call; remove the entry"));
        }
    }
    for name in ["insert_event", "log_task_event", "task_event", "change_event", "event", "TaskChange", "INSERT"] {
        if s.calls.get(name).copied().unwrap_or(0) == 0 {
            problems.push(format!("the scan found no {name} call; was it renamed? Update CALLS in tests/task_event_kinds_test.rs"));
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

#[test]
fn the_frontend_copy_equals_the_list() {
    let path = root().join(JSON_PATH);
    let want = format!("{}\n", serde_json::to_string_pretty(TASK_EVENT_KINDS).unwrap());
    if std::env::var_os("KWANT_WRITE_TASK_EVENT_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    let parsed: Vec<String> = serde_json::from_str(&have).unwrap_or_default();
    assert!(
        parsed == TASK_EVENT_KINDS && have == want,
        "{JSON_PATH} differs from TASK_EVENT_KINDS (src/db/event_kinds.rs). Regenerate it with `{REGENERATE}` and commit it"
    );
}
