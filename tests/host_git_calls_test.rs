//! D4c / S-13 (agents in the VM): kwant starts git on this computer only through
//! `server::dev::git::host_git_command`, which forces `GIT_CONFIG_NOSYSTEM=1` and turns off
//! hooks, fsmonitor and the pager. It is built from its blocking twin
//! `host_git_command_blocking`, whose `Command::new(bin)` is the one line that starts git.
//!
//! #446 should-fix 8: the first version of this test cut each file at its first `#[cfg(test)]`
//! and matched the literal text `Command::new("git")`, so `Command::new(GIT)`, a `"git"` kept in
//! a variable, a path like `"/usr/bin/git"`, a renamed `Command`, or any code after a test-only
//! function in the middle of a file all passed. This version reads `src/` as Rust tokens and
//! proves the claim from both ends:
//!
//! 1. **Every process start is accounted for.** Each `Command::new(…)` outside test code is either
//!    a string literal naming a program that isn't git (a shell must be listed in [`SHELLS_OK`]
//!    with what it runs), or an expression listed in [`DYNAMIC_OK`] with where the program comes
//!    from. The helper's `bin` is the only git entry. A renamed `Command` (`use … Command as X`)
//!    and the other ways to start a program (`libc::exec*`, `posix_spawn`, `CreateProcess…`) fail
//!    outright. A list entry nothing matches fails too, so the lists stay the exact inventory.
//! 2. **The word git goes only where it's safe.** Every string literal that names git (any case,
//!    any folder, with or without `.exe`) must be a match pattern, a comparison, an item in a word
//!    list, an argument to a listed call (the helper, or `run`/`run_with`/`version` in
//!    `server/dev/git.rs`, which have no spawn of their own besides the helper), or the value of a
//!    listed label binding (`const GIT: &str = "git"`, `let provider = match … { … => "git" }`).
//!    `let bin = "git"` fails, so a git name can't travel to a listed dynamic spawn either.
//!
//! The scanner's own tests at the end feed it each way around the old check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `Command::new` arguments that aren't string literals, by file (relative to `src/`) and the
/// argument's tokens as written. Each says where the program comes from and why it isn't git.
const DYNAMIC_OK: &[(&str, &str, &str)] = &[
    ("server/dev/git.rs", "bin", "THE git helper (host_git_command_blocking): adds the safe -c flags and GIT_CONFIG_NOSYSTEM"),
    ("drivers/cli.rs", "bin", "the AI CLI (claude/codex/grok) found by find_cli_binary"),
    ("drivers/cli.rs", "node", "node, to run a CLI's .js entry point"),
    ("documents/reader.rs", "&self.bin", "the document reader's own program (pdftotext etc.)"),
    ("zombie/classify.rs", "&self.bin", "the zombie classifier's model runner"),
    ("tools/shell.rs", "shell_program()?", "the bot's own shell tool under its approval settings (sh/bash/cmd), not a call kwant makes for itself"),
    ("setup.rs", "&command", "a CLI being checked with --version during setup"),
    ("setup.rs", "&chrome", "Chrome for Testing, during setup"),
    ("setup.rs", "&py", "python, during setup"),
    ("setup.rs", "&cmd.argv[_]", "an install command from setup's fixed recipes"),
    ("setup.rs", "npm", "npm, to find its global prefix"),
    ("server/video.rs", "bin", "ffmpeg/ffprobe"),
    ("server/engine_login.rs", "&bin", "the AI CLI's own sign-in command"),
    ("server/dev/scan/gitleaks.rs", "bin", "gitleaks (reads files; never runs git)"),
    ("sandbox/computer.rs", "chrome", "the local Chrome binary"),
    ("sandbox/computer.rs", "&chrome", "the local Chrome binary"),
    ("sandbox/chrome_download.rs", "bin", "the downloaded Chrome, checked with --version"),
    ("router/sidecar.rs", "&cmd.argv[_]", "the router sidecar's install recipe"),
    ("router/sidecar.rs", "bin", "the router sidecar binary"),
    ("mcp/client.rs", "program", "a stdio MCP server the person added (refused in recorded folders, D4/M2)"),
];

/// Shells started by name, with what they run. A shell can run git, so each is listed.
/// Empty: the `powershell` (fsx.rs) and `cmd` (server/gitfile.rs) calls are in their test modules,
/// which the scan skips.
const SHELLS_OK: &[(&str, &str, &str)] = &[];

/// `name = "git"` bindings, and `let name = match … { … => "git" }`: a label, never a program.
const BINDINGS_OK: &[(&str, &str)] = &[
    ("db/dev_mode.rs", "GIT"),
    // A repo's provider label ("git" or "github") for the repos list
    ("server/dev/routes.rs", "provider"),
];

/// Calls a git-naming literal may be an argument of, by file (`*` = any file).
const CALLS_OK: &[(&str, &str)] = &[
    ("*", "host_git_command"),
    ("*", "host_git_command_blocking"),
    ("*", "contains_key"),
    ("*", "starts_with"),
    ("*", "ends_with"),
    // A JSON error's code (`coded(status, "git", message)`)
    ("server/dev/routes.rs", "coded"),
    ("server/dev/git.rs", "run"),
    ("server/dev/git.rs", "run_with"),
    ("server/dev/git.rs", "version"),
];

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "cmd", "powershell", "pwsh"];

#[derive(Debug, Clone, PartialEq)]
enum T {
    Ident(String),
    Str(String),
    Punct(String),
    Other,
}

#[derive(Debug, Clone)]
struct Tok {
    t: T,
    line: usize,
}

/// A small Rust lexer: comments dropped (nested block comments too), string, raw string, byte
/// and char literals read whole, so nothing inside a comment or a string looks like code.
fn lex(src: &str) -> Vec<Tok> {
    let b: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut out) = (0usize, 1usize, Vec::new());
    let ident_start = |c: char| c == '_' || c.is_alphabetic();
    let ident_char = |c: char| c == '_' || c.is_alphanumeric();
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some((value, len, lines)) = string_at(&b, i) {
            out.push(Tok { t: T::Str(value), line });
            line += lines;
            i += len;
        } else if c == '\'' {
            // A char literal ('x', '\n', '\'', '\u{1F600}') or a lifetime ('a)
            let close = if b.get(i + 1) == Some(&'\\') {
                (i + 2..b.len().min(i + 12)).find(|&j| b[j] == '\'' && j > i + 2)
            } else if b.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            match close {
                Some(j) => {
                    out.push(Tok { t: T::Other, line });
                    i = j + 1;
                }
                None => {
                    i += 1;
                    while i < b.len() && ident_char(b[i]) {
                        i += 1;
                    }
                    out.push(Tok { t: T::Other, line });
                }
            }
        } else if ident_start(c) {
            let s = i;
            if c == 'r' && b.get(i + 1) == Some(&'#') && b.get(i + 2).is_some_and(|&c| ident_start(c)) {
                i += 2;
            }
            let from = i;
            while i < b.len() && ident_char(b[i]) {
                i += 1;
            }
            let _ = s;
            out.push(Tok { t: T::Ident(b[from..i].iter().collect()), line });
        } else if c.is_ascii_digit() {
            while i < b.len() && (ident_char(b[i]) || b[i] == '.' && b.get(i + 1).is_some_and(|c| c.is_ascii_digit())) {
                i += 1;
            }
            out.push(Tok { t: T::Other, line });
        } else {
            let two: String = b[i..b.len().min(i + 2)].iter().collect();
            let p = if ["::", "=>", "==", "!=", "->", "&&", "||"].contains(&two.as_str()) { two } else { c.to_string() };
            i += p.chars().count();
            out.push(Tok { t: T::Punct(p), line });
        }
    }
    out
}

/// A string literal starting at `i` ("…", b"…", c"…", r"…", r#"…"#, br#"…"#): its rough value,
/// its length in chars and the newlines in it.
fn string_at(b: &[char], i: usize) -> Option<(String, usize, usize)> {
    let mut j = i;
    if matches!(b.get(j), Some('b') | Some('c')) {
        j += 1;
    }
    let raw = b.get(j) == Some(&'r');
    if raw {
        j += 1;
    }
    let mut hashes = 0;
    while raw && b.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if b.get(j) != Some(&'"') {
        return None;
    }
    // A prefix letter must not be the end of an identifier (`abr"…"` isn't a literal start)
    if i > 0 && (b[i - 1] == '_' || b[i - 1].is_alphanumeric()) && j > i {
        return None;
    }
    j += 1;
    let (mut value, mut lines) = (String::new(), 0);
    while j < b.len() {
        let c = b[j];
        if c == '\n' {
            lines += 1;
        }
        if !raw && c == '\\' {
            if let Some(&n) = b.get(j + 1) {
                value.push(match n {
                    'n' => '\n',
                    't' => '\t',
                    '0' => '\0',
                    other => other,
                });
                if n == '\n' {
                    lines += 1;
                }
            }
            j += 2;
            continue;
        }
        if c == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
            return Some((value, j + 1 + hashes - i, lines));
        }
        value.push(c);
        j += 1;
    }
    None
}

fn is_p(t: Option<&Tok>, p: &str) -> bool {
    matches!(t, Some(Tok { t: T::Punct(x), .. }) if x == p)
}

fn is_i(t: Option<&Tok>, s: &str) -> bool {
    matches!(t, Some(Tok { t: T::Ident(x), .. }) if x == s)
}

/// Index of the innermost group (`(`, `[` or `{`) still open at `k`.
fn open_group(toks: &[Tok], k: usize) -> Option<usize> {
    let mut depth = 0i32;
    for j in (0..k).rev() {
        if let T::Punct(p) = &toks[j].t {
            match p.as_str() {
                ")" | "]" | "}" => depth += 1,
                "(" | "[" | "{" if depth == 0 => return Some(j),
                "(" | "[" | "{" => depth -= 1,
                _ => {}
            }
        }
    }
    None
}

/// For a match arm's value at `k`: `NAME` when the match is `let NAME = match … {` (or
/// `let mut NAME`). Anything else (a match returned, passed or assigned elsewhere) is `None`.
fn match_binding(toks: &[Tok], k: usize) -> Option<String> {
    let body = open_group(toks, k)?;
    if !is_p(toks.get(body), "{") {
        return None;
    }
    // Back over the scrutinee (parentheses and all) to `match`
    let mut depth = 0i32;
    let mut j = body;
    let kw = loop {
        j = j.checked_sub(1)?;
        match &toks[j].t {
            T::Punct(p) if p == ")" || p == "]" => depth += 1,
            T::Punct(p) if p == "(" || p == "[" => depth -= 1,
            T::Punct(p) if depth == 0 && (p == "{" || p == "}" || p == ";") => return None,
            T::Ident(m) if depth == 0 && m == "match" => break j,
            _ => {}
        }
    };
    if !is_p(toks.get(kw.checked_sub(1)?), "=") {
        return None;
    }
    let name = kw.checked_sub(2)?;
    let T::Ident(n) = &toks[name].t else { return None };
    let before = name.checked_sub(1).and_then(|b| toks.get(b));
    (is_i(before, "let") || (is_i(before, "mut") && is_i(toks.get(name.checked_sub(2)?), "let"))).then(|| n.clone())
}

/// Index of the token closing the group opened at `open` (`(`, `[` or `{`).
fn close_of(toks: &[Tok], open: usize) -> usize {
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate().skip(open) {
        if let T::Punct(p) = &t.t {
            match p.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return k;
                    }
                }
                _ => {}
            }
        }
    }
    toks.len() - 1
}

/// Whether the attribute tokens (inside `#[…]`) make the next item test-only: `test`,
/// `tokio::test` (any `…::test`), `cfg(test)`, `cfg(all(test, …))`. `cfg(not(test))` doesn't.
fn test_attr(inner: &[Tok]) -> bool {
    let last_is_test = matches!(inner.last(), Some(Tok { t: T::Ident(x), .. }) if x == "test")
        && inner.iter().all(|t| matches!(&t.t, T::Ident(_)) || matches!(&t.t, T::Punct(p) if p == "::"));
    if last_is_test {
        return true;
    }
    if !is_i(inner.first(), "cfg") {
        return false;
    }
    inner.iter().enumerate().any(|(k, t)| t.t == T::Ident("test".into()) && !(k >= 2 && is_i(inner.get(k - 2), "not")))
}

/// The tokens of non-test code, and the `mod name;` declarations that are test-only.
fn strip_tests(toks: Vec<Tok>) -> (Vec<Tok>, Vec<String>) {
    let (mut out, mut test_mods) = (Vec::new(), Vec::new());
    let mut i = 0;
    // `#![cfg(test)]` at the top: the whole file is test code
    if is_p(toks.first(), "#") && is_p(toks.get(1), "!") && is_p(toks.get(2), "[") {
        let end = close_of(&toks, 2);
        if test_attr(&toks[3..end]) {
            return (Vec::new(), Vec::new());
        }
    }
    while i < toks.len() {
        if is_p(toks.get(i), "#") && is_p(toks.get(i + 1), "[") {
            let end = close_of(&toks, i + 1);
            if test_attr(&toks[i + 2..end]) {
                // Skip any further attributes, then the item: to its `;` or its `{…}`
                let mut k = end + 1;
                while is_p(toks.get(k), "#") && is_p(toks.get(k + 1), "[") {
                    k = close_of(&toks, k + 1) + 1;
                }
                let item_start = k;
                let mut depth = 0i32;
                while k < toks.len() {
                    match &toks[k].t {
                        T::Punct(p) if p == "(" || p == "[" => depth += 1,
                        T::Punct(p) if p == ")" || p == "]" => depth -= 1,
                        T::Punct(p) if p == ";" && depth == 0 => break,
                        T::Punct(p) if p == "{" => {
                            k = close_of(&toks, k);
                            break;
                        }
                        _ => {}
                    }
                    k += 1;
                }
                // `mod name;` (optionally `pub`): that file is test code
                let item: Vec<&Tok> = toks[item_start..k.min(toks.len())].iter().collect();
                if let Some(pos) = item.iter().position(|t| t.t == T::Ident("mod".into())) {
                    if let Some(Tok { t: T::Ident(name), .. }) = item.get(pos + 1) {
                        if is_p(toks.get(k), ";") {
                            test_mods.push(name.clone());
                        }
                    }
                }
                i = k + 1;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    (out, test_mods)
}

/// Lowercased file name without folders and without `.exe`: what a program literal names.
fn program_name(s: &str) -> String {
    let base = s.rsplit(['/', '\\']).next().unwrap_or(s).to_ascii_lowercase();
    base.strip_suffix(".exe").map(str::to_string).unwrap_or(base)
}

fn text(toks: &[Tok]) -> String {
    toks.iter()
        .map(|t| match &t.t {
            T::Ident(s) | T::Punct(s) => s.clone(),
            T::Str(s) => format!("{s:?}"),
            T::Other => "_".into(),
        })
        .collect()
}

#[derive(Default)]
struct Lists<'a> {
    dynamic: &'a [(&'a str, &'a str, &'a str)],
    shells: &'a [(&'a str, &'a str, &'a str)],
    bindings: &'a [(&'a str, &'a str)],
    calls: &'a [(&'a str, &'a str)],
}

#[derive(Default, Debug)]
struct Report {
    problems: Vec<String>,
    /// List entries that matched, as (file, key).
    used: BTreeSet<(String, String)>,
    /// Every git start found: file and line.
    git_starts: Vec<String>,
}

/// Scans one file's non-test tokens.
fn scan_file(file: &str, toks: &[Tok], lists: &Lists, r: &mut Report) {
    let at = |k: usize| format!("{file}:{}", toks[k].line);
    for k in 0..toks.len() {
        // Other ways to start a program
        if let T::Ident(name) = &toks[k].t {
            let after_libc = k >= 2 && is_p(toks.get(k - 1), "::") && (is_i(toks.get(k - 2), "libc") || is_i(toks.get(k - 2), "unistd"));
            let exec_like = name.starts_with("exec") || ["posix_spawn", "posix_spawnp", "fork", "vfork", "system", "popen"].contains(&name.as_str());
            if after_libc && exec_like || name.starts_with("CreateProcess") || name.starts_with("ShellExecute") {
                r.problems.push(format!("{}: starts a program with {name}", at(k)));
            }
            if name == "Command" && is_i(toks.get(k + 1), "as") {
                r.problems.push(format!("{}: Command renamed with `as`, which hides its spawns from this test", at(k)));
            }
        }
        // Command::new(…)
        if is_i(toks.get(k), "Command") && is_p(toks.get(k + 1), "::") && is_i(toks.get(k + 2), "new") && is_p(toks.get(k + 3), "(") {
            let end = close_of(toks, k + 3);
            let arg = &toks[k + 4..end];
            match arg {
                [Tok { t: T::Str(s), .. }] => {
                    let p = program_name(s);
                    if p == "git" {
                        r.problems.push(format!("{}: Command::new({s:?}) starts git without the helper", at(k)));
                    } else if SHELLS.contains(&p.as_str()) {
                        match lists.shells.iter().find(|(f, sh, _)| *f == file && *sh == p) {
                            Some(_) => {
                                r.used.insert((file.to_string(), format!("shell:{p}")));
                            }
                            None => r.problems.push(format!("{}: starts the shell {s:?}, which can run git; list it in SHELLS_OK with what it runs", at(k))),
                        }
                    }
                }
                _ => {
                    let a = text(arg);
                    match lists.dynamic.iter().find(|(f, e, _)| *f == file && *e == a) {
                        Some((_, _, why)) => {
                            r.used.insert((file.to_string(), format!("dyn:{a}")));
                            if why.starts_with("THE git helper") {
                                r.git_starts.push(at(k));
                            }
                        }
                        None => r.problems.push(format!("{}: Command::new({a}) starts a program this test can't name; list it in DYNAMIC_OK with where it comes from", at(k))),
                    }
                }
            }
        }
        // Where a literal naming git goes
        if let T::Str(s) = &toks[k].t {
            if program_name(s) != "git" {
                continue;
            }
            let (prev, next) = (k.checked_sub(1).and_then(|p| toks.get(p)), toks.get(k + 1));
            let pattern = is_p(next, "=>") || is_p(next, "|") || is_p(prev, "|");
            let some_pattern = is_p(prev, "(") && k >= 2 && is_i(toks.get(k - 2), "Some") && is_p(next, ")") && (is_p(toks.get(k + 2), "=>") || is_p(toks.get(k + 2), "|"));
            let compare = is_p(prev, "==") || is_p(prev, "!=") || is_p(next, "==") || is_p(next, "!=");
            // An item of a `[…]` word list: the innermost open group is a `[`
            let in_list = (is_p(prev, "[") || is_p(prev, ",")) && {
                let mut depth = 0i32;
                let mut opener = None;
                for t in toks[..k].iter().rev() {
                    if let T::Punct(p) = &t.t {
                        match p.as_str() {
                            ")" | "]" | "}" => depth += 1,
                            "(" | "[" | "{" if depth == 0 => {
                                opener = Some(p.clone());
                                break;
                            }
                            "(" | "[" | "{" => depth -= 1,
                            _ => {}
                        }
                    }
                }
                opener.as_deref() == Some("[")
            };
            // An argument (any position) of a listed call: the innermost open group is a `(`
            // right after the call's name
            let call = (is_p(prev, "(") || is_p(prev, ",")) && {
                match open_group(toks, k) {
                    Some(o) if is_p(toks.get(o), "(") && o >= 1 => match &toks[o - 1].t {
                        T::Ident(f) => lists.calls.iter().any(|(cf, name)| (*cf == "*" || *cf == file) && name == f),
                        _ => false,
                    },
                    _ => false,
                }
            };
            // A match arm's value, the match bound to a listed label: `let NAME = match … { … => "git" }`
            let arm_label = is_p(prev, "=>") && (is_p(next, ",") || is_p(next, "}")) && {
                match match_binding(toks, k) {
                    Some(n) if lists.bindings.iter().any(|(f, b)| *f == file && *b == n) => {
                        r.used.insert((file.to_string(), format!("bind:{n}")));
                        true
                    }
                    _ => false,
                }
            };
            let binding = is_p(prev, "=") && k >= 4 && {
                // `NAME: &str = "git"` or `NAME = "git"`
                let name = toks[..k - 1].iter().rev().find_map(|t| match &t.t {
                    T::Ident(n) if n != "str" && n != "static" => Some(n.clone()),
                    _ => None,
                });
                match name {
                    Some(n) if lists.bindings.iter().any(|(f, b)| *f == file && *b == n) => {
                        r.used.insert((file.to_string(), format!("bind:{n}")));
                        true
                    }
                    _ => false,
                }
            };
            if !(pattern || some_pattern || compare || in_list || call || binding || arm_label) {
                r.problems.push(format!("{}: {s:?} goes somewhere this test can't follow (a variable, a format, a call it doesn't know); git must start through host_git_command", at(k)));
            }
        }
    }
}

/// Scans every file: `files` are (path relative to `src/`, source).
fn scan(files: &[(String, String)], lists: &Lists) -> Report {
    let mut r = Report::default();
    let mut stripped: BTreeMap<String, Vec<Tok>> = BTreeMap::new();
    let mut test_files: BTreeSet<String> = BTreeSet::new();
    for (path, src) in files {
        let (toks, mods) = strip_tests(lex(src));
        // A module's own folder: `a/mod.rs`, `lib.rs`, `main.rs` → its folder; `a/b.rs` → `a/b/`
        let p = Path::new(path);
        let fname = p.file_name().unwrap().to_string_lossy();
        let dir = if fname == "mod.rs" || (p.parent() == Some(Path::new("")) && (fname == "lib.rs" || fname == "main.rs")) {
            p.parent().unwrap().to_path_buf()
        } else {
            p.with_extension("")
        };
        for m in mods {
            test_files.insert(dir.join(format!("{m}.rs")).to_string_lossy().into_owned());
            test_files.insert(dir.join(&m).join("mod.rs").to_string_lossy().into_owned());
        }
        stripped.insert(path.clone(), toks);
    }
    for (path, toks) in &stripped {
        if !test_files.contains(path) {
            scan_file(path, toks, lists, &mut r);
        }
    }
    r
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn kwant_lists() -> Lists<'static> {
    Lists { dynamic: DYNAMIC_OK, shells: SHELLS_OK, bindings: BINDINGS_OK, calls: CALLS_OK }
}

#[test]
fn only_the_helper_starts_git() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rs_files(&root, &mut paths);
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| (p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read_to_string(p).unwrap()))
        .collect();
    let r = scan(&files, &kwant_lists());
    assert!(r.problems.is_empty(), "host git safety (D4c):\n{}", r.problems.join("\n"));
    // Exactly one git start: the helper's
    assert_eq!(r.git_starts.len(), 1, "{:?}", r.git_starts);
    assert!(r.git_starts[0].starts_with("server/dev/git.rs:"), "{:?}", r.git_starts);
    // Every list entry is still needed: the lists are the inventory, not a pass-anything
    let mut stale = Vec::new();
    for (f, e, _) in DYNAMIC_OK {
        if !r.used.contains(&(f.to_string(), format!("dyn:{e}"))) {
            stale.push(format!("DYNAMIC_OK {f} {e}"));
        }
    }
    for (f, s, _) in SHELLS_OK {
        if !r.used.contains(&(f.to_string(), format!("shell:{s}"))) {
            stale.push(format!("SHELLS_OK {f} {s}"));
        }
    }
    for (f, b) in BINDINGS_OK {
        if !r.used.contains(&(f.to_string(), format!("bind:{b}"))) {
            stale.push(format!("BINDINGS_OK {f} {b}"));
        }
    }
    assert!(stale.is_empty(), "list entries nothing matches any more: {stale:?}");
}

/// The scanner itself: each way around the first version of this test is caught.
mod scanner {
    use super::*;

    fn problems(src: &str) -> Vec<String> {
        let lists = Lists {
            dynamic: &[("server/dev/git.rs", "bin", "THE git helper"), ("a.rs", "program", "listed")],
            shells: &[],
            bindings: &[],
            calls: &[("*", "host_git_command")],
        };
        scan(&[("a.rs".into(), src.into())], &lists).problems
    }

    fn caught(src: &str) {
        assert!(!problems(src).is_empty(), "not caught:\n{src}");
    }

    fn passes(src: &str) {
        let p = problems(src);
        assert!(p.is_empty(), "flagged:\n{src}\n{p:?}");
    }

    #[test]
    fn the_old_literal_is_caught() {
        caught(r#"fn f() { std::process::Command::new("git").arg("status"); }"#);
    }

    #[test]
    fn other_spellings_of_git_are_caught() {
        caught(r#"fn f() { Command::new("/usr/bin/git"); }"#);
        caught(r#"fn f() { Command::new("GIT.EXE"); }"#);
        caught(r##"fn f() { Command::new(r#"git"#); }"##);
        caught(r#"fn f() { tokio::process::Command::new( "C:\\Program Files\\Git\\cmd\\git.exe" ); }"#);
    }

    #[test]
    fn git_through_a_variable_or_constant_is_caught() {
        caught(r#"const GIT: &str = "git"; fn f() { Command::new(GIT); }"#);
        caught(r#"fn f() { let bin = "git"; Command::new(bin); }"#);
        // Even into a spawn the list allows: the name can't get there
        caught(r#"fn f() { let program = "git"; Command::new(program); }"#);
        caught(r#"fn f() { Command::new(format!("{}", "git")); }"#);
        caught(r#"fn f() { Command::new(which("git").unwrap()); }"#);
    }

    #[test]
    fn a_name_built_without_the_word_is_caught() {
        caught(r#"fn f() { Command::new(concat!("gi", "t")); }"#);
        caught(r#"fn f() { Command::new(String::from_utf8(vec![103, 105, 116]).unwrap()); }"#);
    }

    #[test]
    fn a_renamed_command_or_another_spawn_api_is_caught() {
        caught(r#"use std::process::Command as Run; fn f() { Run::new("ls"); }"#);
        caught(r#"fn f() { unsafe { libc::execvp(p, a) }; }"#);
        caught(r#"fn f() { unsafe { libc::posix_spawnp(a, b, c, d, e, g) }; }"#);
        caught(r#"fn f() { unsafe { CreateProcessW(a) }; }"#);
        caught(r#"fn f() { Command::new("sh").arg("-c").arg("git status"); }"#);
    }

    #[test]
    fn code_after_a_test_only_item_is_still_read() {
        // The old version stopped reading at the first #[cfg(test)]
        caught(
            r#"
#[cfg(test)]
fn helper() {}
fn real() { Command::new("git"); }
"#,
        );
        caught(
            r#"
#[test]
fn t() { let x = [0u8; 4]; }
fn real() { Command::new("git"); }
"#,
        );
    }

    #[test]
    fn comments_strings_and_test_modules_are_not_code() {
        passes(
            r#"
// Command::new("git") in a comment
/* and /* nested */ Command::new("git") */
fn f() { let msg = "never Command::new(\"git\") here"; let c = 'x'; let l: &'static str = ""; }
#[cfg(test)]
mod tests { fn t() { std::process::Command::new("git"); } }
#[cfg(all(test, unix))]
mod more { fn t() { Command::new("git"); } }
#[tokio::test]
async fn t2() { Command::new("git"); }
"#,
        );
        // A test-only file declared with `#[cfg(test)] mod x;`
        let lists = Lists { calls: &[], ..Default::default() };
        let files = vec![("lib.rs".to_string(), "#[cfg(test)]\nmod x;\n".to_string()), ("x.rs".to_string(), r#"fn f() { Command::new("git"); }"#.to_string())];
        assert!(scan(&files, &lists).problems.is_empty());
        // …but not cfg(not(test))
        caught(r#"#[cfg(not(test))] fn f() { Command::new("git"); }"#);
    }

    #[test]
    fn safe_uses_of_the_word_pass() {
        passes(
            r#"
fn f(s: &str, w: &[String]) -> bool {
    host_git_command("git");
    match s { "git" => true, "hg" | "git" => true, _ => false };
    let _ = matches!(s, "https" | "git");
    let _ = matches!(Some(s), Some("git") => 1);
    let _ = ["npm:", "git", "http"].iter().any(|p| s.starts_with(p));
    w.iter().any(|x| x == "git") || s == "git" || "git" != s
}
fn g() { Command::new(program); Command::new("docker").arg("ps"); }
"#,
        );
        // A dynamic spawn nobody listed
        caught(r#"fn g() { Command::new(something_else); }"#);
    }

    #[test]
    fn a_label_must_be_listed_and_really_a_label() {
        let lists = Lists {
            dynamic: &[("server/dev/git.rs", "bin", "THE git helper")],
            shells: &[],
            bindings: &[("a.rs", "provider")],
            calls: &[("*", "host_git_command"), ("a.rs", "coded")],
        };
        let check = |src: &str| scan(&[("a.rs".into(), src.into())], &lists).problems;
        // A listed label binding and a listed call, in any argument position
        let ok = check(r#"fn f(x: u8) -> u8 { let provider = match (x, 1) { (0, _) => "git", _ => "github" }; coded(500, "git", provider); 0 }"#);
        assert!(ok.is_empty(), "{ok:?}");
        // The same match bound to another name, returned, or passed on: caught
        for src in [
            r#"fn f(x: u8) { let bin = match x { 0 => "git", _ => "hg" }; }"#,
            r#"fn f(x: u8) -> &'static str { match x { 0 => "git", _ => "hg" } }"#,
            r#"fn f(x: u8) { spawn(match x { 0 => "git", _ => "hg" }); }"#,
            r#"fn f(x: u8) { let provider = if x == 0 { "git" } else { "hg" }; }"#,
            r#"fn f() { other(1, "git"); }"#,
        ] {
            assert!(!check(src).is_empty(), "not caught:\n{src}");
        }
    }
}
