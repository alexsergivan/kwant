//! The risk badge (plan D6, design §5 part 2): a level and the reasons for it, from rules over the
//! diff. Deterministic, no model, no score. Every card shows its size; "Small change, tests nearby"
//! is the reason when nothing else fires.
//!
//! Signals and how much they weigh:
//! - always **High**: a secret in the diff, or a new package that doesn't exist (Juno's scanners);
//! - strong: a security path (auth, passwords, crypto, payments, redaction), a migration, a test
//!   made easier to pass (`tamper`);
//! - regular: deploy files, CI, build config, new packages, no tests near the change, a large
//!   change (more than [`LARGE_LINES`] lines or [`LARGE_FILES`] files), files GitHub sent no diff
//!   for ("Couldn't read N files": the other rules never saw their text);
//! - chips only (a reason, never a level): the first edit to a file in 90 days, and a file outside
//!   the task. `outside_task` comes from the reviewer's intent rows, a model's reading, so it never
//!   moves the level (plan §3.6 item 15, D6).
//!
//! High: two strong, or one strong with two more, or four in all. Medium: any strong or regular.
//! Otherwise Low.

use super::facts::FileDiff;
use super::tamper::{self, Flag, Op, Parsed};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}

/// The card's `risk` (plan §3.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Risk {
    pub level: Level,
    pub reasons: Vec<String>,
}

/// Everything the badge is worked out from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inputs<'a> {
    pub files: &'a [FileDiff],
    /// Files nobody edited in the last 90 days (DevFacts `first_edit_files`).
    pub first_edit_files: &'a [String],
    pub tamper: &'a [Flag],
    /// The intent check found work the task didn't ask for.
    pub outside_task: bool,
    /// Secrets found by the scanner (0 until Juno's lands).
    pub secrets: u32,
    /// New packages that don't exist on their registry (0 until Juno's lands).
    pub missing_packages: u32,
}

/// The chip for work the task didn't ask for.
pub const OUTSIDE_TASK: &str = "Changed a file outside the task";

pub const LARGE_LINES: u64 = 500;
pub const LARGE_FILES: usize = 15;
/// Code lines added with no test near them before it's "No tests near the change".
pub const UNTESTED_MIN: u64 = 10;

pub fn assess(i: &Inputs) -> Risk {
    let parsed = tamper::parse_files(i.files);
    let (mut strong, mut regular) = (0u32, 0u32);
    let mut reasons = Vec::new();
    let mut hard_high = false;

    if i.secrets > 0 {
        hard_high = true;
        reasons.push(if i.secrets == 1 { "Has a secret in the diff".to_string() } else { format!("Has {} secrets in the diff", i.secrets) });
    }
    if i.missing_packages > 0 {
        hard_high = true;
        reasons.push(if i.missing_packages == 1 { "Adds a package that doesn't exist".to_string() } else { format!("Adds {} packages that don't exist", i.missing_packages) });
    }
    let areas = security_areas(i.files);
    if !areas.is_empty() {
        strong += 1;
        reasons.push(format!("Security path: {}", areas.join(", ")));
    }
    if has_migration(&parsed) {
        strong += 1;
        reasons.push("Has a migration".into());
    }
    if !i.tamper.is_empty() {
        strong += 1;
        reasons.push("A test was loosened".into());
    }
    if i.outside_task {
        // A chip only: the model's intent rows never move the level
        reasons.push(OUTSIDE_TASK.into());
    }
    let paths = || i.files.iter().map(|f| f.path.as_str()).chain(i.files.iter().filter_map(|f| f.previous_path.as_deref()));
    for (hit, chip) in [(paths().any(is_deploy_path), "Changes deploy files"), (paths().any(is_ci_path), "Changes CI"), (paths().any(is_build_config_path), "Changes build config")] {
        if hit {
            regular += 1;
            reasons.push(chip.into());
        }
    }
    let pkgs = new_packages(i.files);
    if !pkgs.is_empty() {
        regular += 1;
        reasons.push(if pkgs.len() == 1 { "Adds 1 package".into() } else { format!("Adds {} packages", pkgs.len()) });
    }
    let code_changed = parsed.iter().any(|f| is_code(f) && f.lines().any(|l| l.op != Op::Ctx && !l.test));
    let untested = untested_lines(&parsed);
    if untested >= UNTESTED_MIN {
        regular += 1;
        reasons.push("No tests near the change".into());
    }
    // GitHub sends no patch for very large files: the rules couldn't read them, so say so (and
    // never "Small change" or "no code changed" over text nobody saw)
    let unread = unread_files(i.files);
    if !unread.is_empty() {
        regular += 1;
        reasons.push(unread_reason(unread.len()));
    }
    let (add, del) = i.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    if add + del > LARGE_LINES || i.files.len() > LARGE_FILES {
        regular += 1;
    }
    let added: BTreeSet<&str> = i.files.iter().filter(|f| f.status == "added").map(|f| f.path.as_str()).collect();
    let first: Vec<&str> = i.first_edit_files.iter().map(String::as_str).filter(|p| !added.contains(p)).collect();
    if let Some(f) = first.first() {
        let name = f.rsplit('/').next().unwrap_or(f);
        reasons.push(if first.len() == 1 { format!("First edit to {name}") } else { format!("First edit to {name} +{} more", first.len() - 1) });
    }

    let level = if hard_high || strong >= 2 || (strong >= 1 && strong + regular >= 3) || strong + regular >= 4 {
        Level::High
    } else if strong + regular >= 1 {
        Level::Medium
    } else {
        Level::Low
    };
    if reasons.is_empty() {
        reasons.push(
            match (code_changed, untested) {
                (false, _) => "Small change, no code changed",
                (true, 0) => "Small change, tests nearby",
                (true, _) => "Small change",
            }
            .into(),
        );
    }
    reasons.push(size_chip(add, del, i.files.len()));
    Risk { level, reasons }
}

/// Files with changed lines but no patch, from git or the forge alike: too large to diff, or a
/// forge that sends no patch at all (Gitea's fallback to its files list). Binary files (no lines)
/// aren't counted: there's no text to read.
pub fn unread_files(files: &[FileDiff]) -> Vec<&str> {
    files.iter().filter(|f| f.patch.is_none() && f.additions + f.deletions > 0).map(|f| f.path.as_str()).collect()
}

/// "Couldn't read 2 files (no diff available)". It names no host (the files may have come from
/// git, GitHub, GitLab, Gitea or Forgejo) and no single cause: size is only one of them.
pub fn unread_reason(n: usize) -> String {
    format!("Couldn't read {n} file{} (no diff available)", if n == 1 { "" } else { "s" })
}

/// "+27 −9 in 2 files" (a real minus sign, as the design writes it).
pub fn size_chip(add: u64, del: u64, files: usize) -> String {
    format!("+{add} \u{2212}{del} in {files} file{}", if files == 1 { "" } else { "s" })
}

fn is_code(f: &Parsed) -> bool {
    tamper::is_code_path(f.path) && !f.test_file && !f.snapshot
}

// ---------------------------------------------------------------------------------------------
// Paths

/// The words of a path: split on punctuation and camelCase, lowercased, plus each pair of
/// neighbours joined (`sign_in` → `signin`, `PassWord` → `password`).
fn path_words(path: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in path.chars() {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        cur.push(c.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let pairs: Vec<String> = words.windows(2).map(|w| format!("{}{}", w[0], w[1])).collect();
    words.extend(pairs);
    words
}

/// Words that name a security area exactly (prefix matching would catch "author").
const AUTH_EXACT: [&str; 13] =
    ["auth", "oauth", "oauth2", "authn", "authz", "authentication", "authorization", "authorize", "login", "logout", "signin", "signup", "sso"];
/// The words that name the other areas, whole words only (word starts would catch "secretary",
/// "vaulted"): each stem and its common forms.
const AREA_WORDS: [(&str, &[&str]); 4] = [
    ("passwords", &["password", "passwords", "passwd", "credential", "credentials", "keychain", "secret", "secrets", "vault", "vaults"]),
    ("crypto", &["crypto", "cryptography", "encrypt", "encrypted", "encryption", "decrypt", "decrypted", "decryption", "cipher", "ciphers", "hmac"]),
    ("payments", &["payment", "payments", "billing", "stripe", "checkout", "invoice", "invoices", "invoicing", "payout", "payouts", "refund", "refunds"]),
    ("redaction", &["redact", "redacted", "redaction", "redactor"]),
];

/// Images, video, fonts and other media: a file named `signin.webp` isn't sign-in code.
fn is_media_path(path: &str) -> bool {
    let name = file_name(path);
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    ["png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "ico", "bmp", "mp4", "webm", "mov", "mp3", "wav", "ogg", "woff", "woff2", "ttf", "otf", "pdf"].contains(&ext)
}

/// The security areas the changed code touches (docs, tests and media don't count), in a fixed order.
pub fn security_areas(files: &[FileDiff]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for f in files {
        if tamper::is_test_path(&f.path) || is_doc_path(&f.path) || is_media_path(&f.path) {
            continue;
        }
        let words = path_words(&f.path);
        if words.iter().any(|w| AUTH_EXACT.contains(&w.as_str())) && !out.contains(&"auth") {
            out.push("auth");
        }
        // `git/checkout.rs` checks out a branch; it takes no money
        let git = words.iter().any(|w| w == "git");
        for (area, names) in AREA_WORDS {
            let hit = words.iter().any(|w| names.contains(&w.as_str()) && !(git && w == "checkout"));
            if hit && !out.contains(&area) {
                out.push(area);
            }
        }
    }
    let order = ["auth", "passwords", "crypto", "payments", "redaction"];
    out.sort_by_key(|a| order.iter().position(|o| o == a));
    out
}

fn is_doc_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("docs/") || p.contains("/docs/") || [".md", ".mdx", ".txt", ".rst", ".adoc"].iter().any(|e| p.ends_with(e))
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase()
}

pub fn is_ci_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = file_name(path);
    p.starts_with(".github/workflows/")
        || p.starts_with(".github/actions/")
        || p.starts_with(".circleci/")
        || p.starts_with(".buildkite/")
        || name == ".gitlab-ci.yml"
        || name == "jenkinsfile"
        || name == ".travis.yml"
        || name == "azure-pipelines.yml"
        || name == "bitbucket-pipelines.yml"
        || name == "dependabot.yml"
}

pub fn is_deploy_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = file_name(path);
    let dirs: Vec<&str> = p.split('/').collect();
    let dirs = &dirs[..dirs.len().saturating_sub(1)];
    name == "dockerfile"
        || name.starts_with("dockerfile.")
        || name.ends_with(".dockerfile")
        || name.starts_with("docker-compose")
        || name.starts_with("compose.y")
        || ["fly.toml", "wrangler.toml", "wrangler.json", "wrangler.jsonc", "netlify.toml", "vercel.json", "procfile", "app.yaml", "render.yaml", "railway.json", "nginx.conf", "caddyfile"].contains(&name.as_str())
        || name.ends_with(".tf")
        || name.ends_with(".tfvars")
        || name.ends_with(".service")
        || dirs.iter().any(|d| ["k8s", "kubernetes", "helm", "charts", "deploy", "deployment", "infra", "terraform", "ansible", "docker"].contains(d))
}

pub fn is_build_config_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = file_name(path);
    !tamper::is_test_path(path)
        && (name.starts_with("tsconfig")
            || name.contains(".config.")
            || name == "build.rs"
            || name == "makefile"
            || name == "justfile"
            || name.starts_with("rust-toolchain")
            || name.starts_with(".eslintrc")
            || name == ".babelrc"
            || name == ".npmrc"
            || name == "setup.cfg"
            || p.ends_with(".cargo/config.toml")
            || p.ends_with(".cargo/config"))
}

fn is_lockfile(path: &str) -> bool {
    let name = file_name(path);
    name.ends_with(".lock") || name.ends_with("-lock.json") || name.ends_with("-lock.yaml") || name == "go.sum" || name == "npm-shrinkwrap.json"
}

// ---------------------------------------------------------------------------------------------
// Migrations

const DDL: [&str; 11] =
    ["CREATE TABLE", "CREATE INDEX", "CREATE UNIQUE INDEX", "CREATE VIEW", "CREATE TRIGGER", "ALTER TABLE", "DROP TABLE", "DROP INDEX", "DROP COLUMN", "ADD COLUMN", "RENAME COLUMN"];

fn is_migration_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = file_name(path);
    p.split('/').any(|d| d == "migrations" || d == "migrate" || d == "alembic" || d == "migration")
        || name.ends_with(".sql")
        || name == "schema.prisma"
        || name == "schema.rb"
        || name == "structure.sql"
}

/// A migration file changed, or code added a schema change (DDL in a string, not in a test).
pub fn has_migration(files: &[Parsed]) -> bool {
    files.iter().any(|f| {
        if tamper::is_test_path(f.path) || is_doc_path(f.path) {
            return false;
        }
        if is_migration_path(f.path) {
            return true;
        }
        f.lines().any(|l| {
            l.op == Op::Add && !l.test && (l.text.contains('"') || l.text.contains('\'') || l.text.contains('`')) && !l.text.trim_start().starts_with("//") && has_ddl(l.text)
        })
    })
}

/// DDL in `text`: upper case as it stands, or lower case (`create table if not exists t`) when a
/// name follows. Mixed case ("Add column", "Drop index card") reads as a label, not SQL.
pub fn has_ddl(text: &str) -> bool {
    DDL.iter().any(|d| {
        if text.contains(d) {
            return true;
        }
        let lower = d.to_ascii_lowercase();
        text.match_indices(lower.as_str()).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let rest = &text[i + lower.len()..];
            let gap = rest.len() - rest.trim_start().len();
            before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
                && gap > 0
                && rest.trim_start().chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || "_\"`[".contains(c))
        })
    })
}

// ---------------------------------------------------------------------------------------------
// New packages

/// Which manifest a file is, for the package check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Npm,
    Crates,
    PyPI,
    Go,
    Gems,
}

/// A package the diff adds to a manifest (Juno's scanner checks it against its registry).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NewPackage {
    pub ecosystem: &'static str,
    pub name: String,
    pub file: String,
}

fn ecosystem_of(path: &str) -> Option<Ecosystem> {
    let name = file_name(path);
    if is_lockfile(path) {
        return None;
    }
    match name.as_str() {
        "package.json" => Some(Ecosystem::Npm),
        "cargo.toml" => Some(Ecosystem::Crates),
        "pyproject.toml" | "pipfile" => Some(Ecosystem::PyPI),
        "go.mod" => Some(Ecosystem::Go),
        "gemfile" => Some(Ecosystem::Gems),
        n if n.starts_with("requirements") && n.ends_with(".txt") => Some(Ecosystem::PyPI),
        _ => None,
    }
}

const NPM_DEP_SECTIONS: [&str; 5] = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies", "bundledDependencies"];
const NPM_OTHER_KEYS: [&str; 22] = [
    "name", "version", "description", "main", "module", "types", "typings", "license", "author", "private", "type", "packageManager", "homepage", "repository", "node", "npm", "pnpm", "yarn", "browser", "sideEffects", "bin", "workspaces",
];
const TOML_OTHER_KEYS: [&str; 22] = [
    "name", "version", "edition", "authors", "description", "license", "repository", "homepage", "readme", "rust-version", "publish", "default-run", "build", "keywords", "categories", "documentation", "resolver", "members", "exclude", "include", "requires-python", "python",
];

fn quoted_key(t: &str) -> Option<(&str, &str)> {
    let t = t.trim().strip_prefix('"')?;
    let (k, rest) = t.split_once('"')?;
    Some((k, rest.trim_start().strip_prefix(':')?.trim()))
}

fn version_like(v: &str) -> bool {
    let v = v.trim().trim_start_matches('"');
    v.starts_with(|c: char| c.is_ascii_digit() || "^~*><=".contains(c)) || ["workspace:", "npm:", "file:", "link:", "git", "http", "latest", "next"].iter().any(|p| v.starts_with(p))
}

/// The package a manifest line names, given the section it sits in (when the diff shows it).
fn package_on(eco: Ecosystem, line: &str, section: Option<&str>) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
        return None;
    }
    match eco {
        Ecosystem::Npm => {
            let (k, v) = quoted_key(t)?;
            let in_deps = section.map(|s| NPM_DEP_SECTIONS.contains(&s));
            match in_deps {
                Some(true) => (!v.starts_with('{')).then(|| k.to_string()),
                Some(false) => None,
                None => (!NPM_OTHER_KEYS.contains(&k) && !NPM_DEP_SECTIONS.contains(&k) && version_like(v)).then(|| k.to_string()),
            }
        }
        Ecosystem::Crates | Ecosystem::PyPI if t.starts_with('[') => {
            // `[dependencies.serde]` names one
            let h = t.trim_matches(['[', ']']);
            let (sec, name) = h.rsplit_once('.')?;
            (sec.ends_with("dependencies")).then(|| name.trim_matches('"').to_string())
        }
        Ecosystem::Crates | Ecosystem::PyPI => {
            let in_deps = section.map(|s| s.ends_with("dependencies") || s.ends_with("dev-packages") || s.ends_with("packages") || s.ends_with("requires"));
            // pyproject's `dependencies = [ "requests>=2", ... ]` items
            if let Some(item) = t.strip_prefix('"').or_else(|| t.strip_prefix('\'')) {
                if in_deps == Some(false) && section != Some("project") {
                    return None;
                }
                let name: String = item.chars().take_while(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect();
                return (!name.is_empty() && eco == Ecosystem::PyPI).then_some(name);
            }
            let (k, v) = t.split_once('=')?;
            let k = k.trim().trim_matches('"');
            if k.is_empty() || k.contains(' ') || k == "dependencies" || k == "optional-dependencies" {
                return None;
            }
            match in_deps {
                Some(true) => Some(k.to_string()),
                Some(false) => None,
                None => (!TOML_OTHER_KEYS.contains(&k) && (version_like(v) || v.trim().starts_with('{'))).then(|| k.to_string()),
            }
        }
        Ecosystem::Go => {
            if t.contains("// indirect") || t.starts_with("module ") || t.starts_with("go ") || t == ")" || t.starts_with("toolchain") {
                return None;
            }
            let t = t.strip_prefix("require ").unwrap_or(t);
            let mut parts = t.split_whitespace();
            let (name, ver) = (parts.next()?, parts.next()?);
            (ver.starts_with('v') && name.contains('.') && section != Some("replace") && section != Some("exclude")).then(|| name.to_string())
        }
        Ecosystem::Gems => {
            let rest = t.strip_prefix("gem ")?;
            Some(rest.trim().trim_start_matches(['"', '\'']).split(['"', '\'']).next()?.to_string())
        }
    }
}

/// The section a manifest line opens, if it opens one.
fn opens_section(eco: Ecosystem, line: &str) -> Option<String> {
    let t = line.trim();
    match eco {
        Ecosystem::Npm => {
            let (k, v) = quoted_key(t)?;
            v.starts_with('{').then(|| k.to_string())
        }
        Ecosystem::Crates | Ecosystem::PyPI => {
            if t.starts_with('[') && !t.starts_with("[[") {
                let h = t.trim_matches(['[', ']']);
                // `[target.'cfg(unix)'.dependencies]` is a dependencies section
                return Some(if h.ends_with("dependencies") { "dependencies".into() } else { h.to_string() });
            }
            let (k, v) = t.split_once('=')?;
            let k = k.trim();
            (v.trim() == "[" || v.trim().starts_with('[') && !v.contains(']')).then(|| if k == "dependencies" || k.ends_with("requires") { "dependencies".into() } else { k.to_string() })
        }
        Ecosystem::Go => {
            let w = t.strip_suffix('(')?.trim();
            Some(w.to_string())
        }
        Ecosystem::Gems => None,
    }
}

/// The packages the diff adds: named on an added manifest line and not on a removed one (a
/// version bump removes and adds the same name). Lockfiles don't count.
pub fn new_packages(files: &[FileDiff]) -> Vec<NewPackage> {
    let mut out = BTreeSet::new();
    for f in files {
        let Some(eco) = ecosystem_of(&f.path) else { continue };
        let (mut added, mut removed) = (BTreeSet::new(), BTreeSet::new());
        let requirements = eco == Ecosystem::PyPI && file_name(&f.path).starts_with("requirements");
        for h in tamper::hunks(f.patch.as_deref().unwrap_or("")) {
            let mut section: Option<String> = None;
            for l in &h.lines {
                let name = if requirements {
                    let t = l.text.trim();
                    (!t.is_empty() && !t.starts_with('#') && !t.starts_with('-')).then(|| t.chars().take_while(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect::<String>())
                } else {
                    package_on(eco, l.text, section.as_deref())
                };
                if let Some(s) = opens_section(eco, l.text) {
                    section = Some(s);
                } else if matches!(l.text.trim(), "}" | "}," | ")" | "]" | "],") {
                    section = None;
                }
                let Some(name) = name.filter(|n| !n.is_empty()) else { continue };
                match l.op {
                    Op::Add => added.insert(name),
                    Op::Del => removed.insert(name),
                    Op::Ctx => false,
                };
            }
        }
        let label = match eco {
            Ecosystem::Npm => "npm",
            Ecosystem::Crates => "crates",
            Ecosystem::PyPI => "pypi",
            Ecosystem::Go => "go",
            Ecosystem::Gems => "rubygems",
        };
        for name in added.difference(&removed) {
            out.insert(NewPackage { ecosystem: label, name: name.clone(), file: f.path.clone() });
        }
    }
    out.into_iter().collect()
}

// ---------------------------------------------------------------------------------------------
// Tests near the change

/// Code lines added in files with no test change near them.
fn untested_lines(files: &[Parsed]) -> u64 {
    let tests: Vec<&Parsed> = files.iter().filter(|f| f.test_file && f.lines().any(|l| l.op != Op::Ctx)).collect();
    files
        .iter()
        .filter(|f| is_code(f))
        .filter(|f| {
            let inline = f.lines().any(|l| l.test && l.op != Op::Ctx);
            let area = tamper::area_of(f.path);
            !inline && !tests.iter().any(|t| tamper::test_covers(t.path, &area, &[f.path]))
        })
        .map(|f| f.lines().filter(|l| l.op == Op::Add && !l.test).count() as u64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::super::facts::DevFacts;
    use super::super::tamper::Kind;
    use super::*;

    fn file(path: &str, status: &str, patch: &str) -> FileDiff {
        let additions = patch.lines().filter(|l| l.starts_with('+')).count() as u64;
        let deletions = patch.lines().filter(|l| l.starts_with('-')).count() as u64;
        FileDiff { path: path.into(), status: status.into(), additions, deletions, patch: Some(patch.into()), previous_path: None }
    }
    fn modified(path: &str, patch: &str) -> FileDiff {
        file(path, "modified", patch)
    }
    /// `n` lines of new code in `path`.
    fn code(path: &str, n: usize) -> FileDiff {
        let body: String = (0..n).map(|i| format!("+    let v{i} = step({i});\n")).collect();
        modified(path, &format!("@@ -10,2 +10,{} @@ fn run() {{\n     let a = 1;\n{body}     a\n", n + 2))
    }
    fn test_edit(path: &str) -> FileDiff {
        modified(path, "@@ -1,1 +1,2 @@\n test('x', () => {\n+  assert.equal(run(), 2);\n")
    }
    fn risk(files: &[FileDiff]) -> Risk {
        assess(&Inputs { files, ..Default::default() })
    }
    fn flag() -> Flag {
        Flag { kind: Kind::Loosened, file: "tests/a_test.rs".into(), line: 3, excerpt: String::new(), why: String::new() }
    }
    fn fixture(n: u32) -> DevFacts {
        serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr{n}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
    }

    #[test]
    fn size_is_always_the_last_reason() {
        let r = risk(&[code("src/runner.rs", 3), test_edit("tests/runner_test.rs")]);
        assert_eq!(r.level, Level::Low);
        assert_eq!(r.reasons, vec!["Small change, tests nearby".to_string(), "+4 \u{2212}0 in 2 files".into()]);
        assert_eq!(size_chip(1, 0, 1), "+1 \u{2212}0 in 1 file");
        let r = risk(&[modified("docs/x.md", "@@ -1,1 +1,1 @@\n-a\n+b\n")]);
        assert_eq!(r.reasons, vec!["Small change, no code changed".to_string(), "+1 \u{2212}1 in 1 file".into()]);
    }

    #[test]
    fn security_paths_name_their_area() {
        for (path, area) in [
            ("src/server/auth.rs", "auth"),
            ("frontend/src/components/SignIn.tsx", "auth"),
            ("src/oauth/callback.rs", "auth"),
            ("src/vault/store.rs", "passwords"),
            ("src/server/password_reset.rs", "passwords"),
            ("src/crypto/seal.rs", "crypto"),
            ("src/server/stripe_webhook.rs", "payments"),
            ("frontend/src/billing/Plan.tsx", "payments"),
            ("src/server/activity/redaction.rs", "redaction"),
        ] {
            let r = risk(&[code(path, 3)]);
            assert_eq!(r.reasons[0], format!("Security path: {area}"), "{path}");
            assert_eq!(r.level, Level::Medium, "{path}");
        }
        // Lookalikes, tests and docs don't count
        for path in ["src/server/author.rs", "src/db/tokens.rs", "src/server/session.rs", "tests/auth_test.rs", "docs/auth.md", "frontend/src/Authority.tsx"] {
            assert!(security_areas(&[code(path, 3)]).is_empty(), "{path}");
        }
        assert_eq!(security_areas(&[code("src/redact.rs", 1), code("src/auth.rs", 1)]), vec!["auth", "redaction"]);
    }

    #[test]
    fn a_migration_is_a_file_or_new_schema_sql() {
        assert!(risk(&[file("migrations/20261105_dev.sql", "added", "@@ -0,0 +1,1 @@\n+create table x (id int);\n")]).reasons.contains(&"Has a migration".into()));
        let ddl = modified("src/db/reactions.rs", "@@ -60,1 +60,2 @@ impl Database {\n     let a = 1;\n+        \"ALTER TABLE message_reactions ADD COLUMN told INTEGER NOT NULL DEFAULT 0\",\n");
        let r = risk(&[ddl]);
        assert_eq!(r.reasons[0], "Has a migration");
        assert_eq!(r.level, Level::Medium);
        // SQL in a test, a comment, or prose isn't a migration
        let not = [
            modified("src/db/x.rs", "@@ -400,1 +400,2 @@ mod tests {\n     #[test]\n+        db.exec(\"CREATE TABLE t (a)\");\n"),
            modified("src/db/x.rs", "@@ -1,1 +1,2 @@\n+    // ALTER TABLE runs once at start\n"),
            modified("docs/db.md", "@@ -1,1 +1,2 @@\n+Run `ALTER TABLE` by hand.\n"),
            modified("src/db/x.rs", "@@ -1,1 +1,2 @@\n+    let alter_table = 1;\n"),
        ];
        assert!(!has_migration(&tamper::parse_files(&not)));
    }

    /// Round 3 (Rigel's DDL row, feeds `needs_backup`): `create table` and `alter table` count as a
    /// migration in upper or lower case; a UI label like "Add column" doesn't.
    #[test]
    fn lowercase_ddl_is_a_migration() {
        for line in [
            "+        sqlx::query(\"create table if not exists foo (id text)\").execute(&pool).await?;",
            "+        sqlx::query(\"alter table kwants add column foo text\").execute(&pool).await?;",
            "+    r#\"create unique index idx on t(a)\"#,",
            "+        sqlx::query(\"drop table old\")",
        ] {
            let f = modified("src/db/spaces.rs", &format!("@@ -1,1 +1,2 @@\n ctx\n{line}\n"));
            assert!(risk(&[f]).reasons.contains(&"Has a migration".into()), "{line}");
        }
        for line in ["+    <button title=\"Add column\">", "+    let msg = \"create tables of contents\";", "+    label: 'Drop index card',", "+    title: \"Create Table view\","] {
            let f = modified("frontend/src/Board.tsx", &format!("@@ -1,1 +1,2 @@\n ctx\n{line}\n"));
            assert!(!risk(&[f]).reasons.contains(&"Has a migration".into()), "{line}");
        }
        assert!(has_ddl("ALTER TABLE x") && has_ddl("alter table x") && has_ddl("ALTER TABLE") && !has_ddl("alter table"));
    }

    /// 0.7.7: the reason names no host and no cause. A GitLab repo read with plain git said "no
    /// diff from GitHub"; then "too large to diff" showed on a +2/−1 type change (Rigel's F-1),
    /// and Gitea's fallback to its files list sends no patch whatever the size.
    #[test]
    fn the_unread_reason_names_no_host_and_no_cause() {
        assert_eq!(unread_reason(1), "Couldn't read 1 file (no diff available)");
        assert_eq!(unread_reason(3), "Couldn't read 3 files (no diff available)");
        for n in [1, 2] {
            let r = unread_reason(n);
            for word in ["GitHub", "GitLab", "Gitea", "Forgejo", "git", "large", "size"] {
                assert!(!r.contains(word), "{r}");
            }
        }
    }

    /// Round 3 (V-T5): GitHub sends no patch for very large files. The card says it couldn't read
    /// them and never calls the change small or code-free.
    #[test]
    fn a_file_github_sent_no_diff_for_is_never_a_small_change() {
        let unread = |path: &str, add: u64, del: u64| FileDiff { path: path.into(), status: "modified".into(), additions: add, deletions: del, patch: None, previous_path: None };
        // A test file losing 300 lines, unread: at least Medium, and said
        let r = risk(&[unread("tests/a_test.rs", 0, 300)]);
        assert_eq!(r.level, Level::Medium);
        assert_eq!(r.reasons, vec!["Couldn't read 1 file (no diff available)".to_string(), "+0 \u{2212}300 in 1 file".into()]);
        // A big source file, unread: large plus unread
        let r = risk(&[unread("src/a.rs", 900, 0)]);
        assert!(r.level >= Level::Medium);
        assert!(r.reasons.contains(&"Couldn't read 1 file (no diff available)".into()));
        // Next to a small read change: still no "Small change"
        let r = risk(&[code("src/runner.rs", 2), test_edit("tests/runner_test.rs"), unread("src/big.rs", 4, 0), unread("tests/big_test.rs", 1, 9)]);
        assert!(r.reasons.contains(&"Couldn't read 2 files (no diff available)".into()), "{:?}", r.reasons);
        assert!(r.reasons.iter().all(|x| !x.starts_with("Small change") && !x.contains("no code changed")), "{:?}", r.reasons);
        // A binary file (no lines) has no text to read: not counted
        let png = FileDiff { path: "site/a.png".into(), status: "modified".into(), additions: 0, deletions: 0, patch: None, previous_path: None };
        assert!(unread_files(std::slice::from_ref(&png)).is_empty());
        assert_eq!(risk(&[png]).level, Level::Low);
    }

    /// Round 3 (Rigel's security-path false positives): whole words only, and media files never.
    #[test]
    fn lookalike_paths_and_media_are_not_security_paths() {
        for path in ["src/secretary.rs", "src/git/checkout.rs", "src/vaulted.rs", "site/assets/media/thumb-v69-signin-1280.webp", "frontend/public/login-hero.png", "src/encryptionless.rs"] {
            assert!(security_areas(&[code(path, 3)]).is_empty(), "{path}: {:?}", security_areas(&[code(path, 3)]));
        }
        for (path, area) in [("frontend/src/checkout/Cart.tsx", "payments"), ("src/secrets.rs", "passwords"), ("src/server/credentials.rs", "passwords"), ("frontend/src/components/ClaudeSignIn.tsx", "auth"), ("src/encryption.rs", "crypto")] {
            assert_eq!(security_areas(&[code(path, 3)]), vec![area], "{path}");
        }
    }

    #[test]
    fn ci_deploy_and_build_config_files_have_their_own_reasons() {
        let r = risk(&[modified(".github/workflows/ci.yml", "@@ -1,1 +1,1 @@\n-a\n+b\n")]);
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::Medium, "Changes CI"));
        for p in ["Dockerfile", "deploy/kwant.service", "fly.toml", "infra/main.tf", "docker-compose.yml", "site/wrangler.toml"] {
            assert!(is_deploy_path(p), "{p}");
        }
        for p in ["src/server/deploy.rs", "frontend/src/DeployButton.tsx", "docs/deploy.md"] {
            assert!(!is_deploy_path(p), "{p}");
        }
        let r = risk(&[modified("frontend/vite.config.ts", "@@ -1,1 +1,1 @@\n-a\n+b\n")]);
        assert_eq!(r.reasons[0], "Changes build config");
        assert!(!is_build_config_path("frontend/tests/vitest.config.test.ts"));
    }

    #[test]
    fn new_packages_are_counted_and_version_bumps_are_not() {
        let pkg = modified(
            "frontend/package.json",
            "@@ -10,6 +10,8 @@\n   \"dependencies\": {\n-    \"react\": \"^18.2.0\",\n+    \"react\": \"^18.3.1\",\n+    \"left-padx\": \"^1.0.0\",\n     \"zod\": \"^3.0.0\"\n   },\n   \"scripts\": {\n+    \"lint\": \"eslint .\",\n",
        );
        let cargo = modified("Cargo.toml", "@@ -20,3 +20,5 @@ edition = \"2021\"\n [dependencies]\n+regex = \"1\"\n+serde = { version = \"1\", features = [\"derive\"] }\n-serde = \"1\"\n");
        let req = modified("requirements.txt", "@@ -1,1 +1,3 @@\n flask==3.0\n+requests>=2.31\n+# a comment\n");
        let gomod = modified("go.mod", "@@ -3,3 +3,5 @@ go 1.22\n require (\n+\tgithub.com/acme/demo v1.2.0\n+\tgolang.org/x/sys v0.1.0 // indirect\n )\n");
        let got: Vec<(&str, String)> = new_packages(&[pkg.clone(), cargo, req, gomod]).into_iter().map(|p| (p.ecosystem, p.name)).collect();
        assert_eq!(got, vec![("crates", "regex".into()), ("go", "github.com/acme/demo".into()), ("npm", "left-padx".into()), ("pypi", "requests".into())]);
        let r = risk(&[pkg]);
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::Medium, "Adds 1 package"));
        // The package's own version, scripts, engines and lockfiles aren't packages
        let not = [
            modified("package.json", "@@ -1,3 +1,3 @@\n   \"name\": \"kwant-frontend\",\n-  \"version\": \"0.6.11\",\n+  \"version\": \"0.6.12\",\n"),
            modified("Cargo.toml", "@@ -1,3 +1,3 @@\n [package]\n-version = \"0.6.11\"\n+version = \"0.6.12\"\n"),
            modified("frontend/package-lock.json", "@@ -1,1 +1,2 @@\n+    \"node_modules/evil\": {\n"),
            modified("package.json", "@@ -30,2 +30,3 @@\n   \"engines\": {\n+    \"pnpm\": \">=9\"\n   }\n"),
        ];
        assert!(new_packages(&not).is_empty(), "{:?}", new_packages(&not));
    }

    #[test]
    fn code_with_no_tests_near_it_is_a_reason() {
        let r = risk(&[code("src/server/reactions.rs", 12)]);
        assert_eq!(r.reasons, vec!["No tests near the change".to_string(), "+12 \u{2212}0 in 1 file".into()]);
        assert_eq!(r.level, Level::Medium);
        // Covered: a test in the same area, a top-level test named after it, inline tests
        assert_eq!(risk(&[code("frontend/src/components/Sidebar.tsx", 12), test_edit("frontend/tests/polish063.test.mjs")]).level, Level::Low);
        assert_eq!(risk(&[code("src/server/reactions.rs", 12), test_edit("tests/reactions_replies_test.rs")]).level, Level::Low);
        let inline = modified("src/db/x.rs", &format!("{}@@ -400,1 +412,2 @@ mod tests {{\n     #[test]\n+    fn t() {{}}\n", code("src/db/x.rs", 12).patch.unwrap()));
        assert_eq!(risk(&[inline]).level, Level::Low);
        // A little untested code is a small change, not a reason
        assert_eq!(risk(&[code("src/server/reactions.rs", 4)]).reasons[0], "Small change");
        // A test elsewhere doesn't cover it
        assert_eq!(risk(&[code("src/server/reactions.rs", 12), test_edit("frontend/tests/ia.test.mjs")]).reasons[0], "No tests near the change");
    }

    #[test]
    fn first_edits_are_a_reason_but_not_a_level() {
        let files = [code("src/a.rs", 3), test_edit("tests/a_test.rs"), file("src/new.rs", "added", "@@ -0,0 +1,1 @@\n+fn x() {}\n")];
        let firsts = ["src/a.rs".to_string(), "src/new.rs".into()];
        let r = assess(&Inputs { files: &files, first_edit_files: &firsts, ..Default::default() });
        assert_eq!(r.level, Level::Low);
        assert_eq!(r.reasons[0], "First edit to a.rs");
        let firsts = ["src/a.rs".to_string(), "tests/a_test.rs".into()];
        assert_eq!(assess(&Inputs { files: &files, first_edit_files: &firsts, ..Default::default() }).reasons[0], "First edit to a.rs +1 more");
    }

    #[test]
    fn tamper_raises_the_level_and_outside_task_is_only_a_chip() {
        let files = [code("src/a.rs", 3), test_edit("tests/a_test.rs")];
        let flags = [flag()];
        let r = assess(&Inputs { files: &files, tamper: &flags, ..Default::default() });
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::Medium, "A test was loosened"));
        let r = assess(&Inputs { files: &files, outside_task: true, ..Default::default() });
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::Low, OUTSIDE_TASK));
    }

    /// D6 / plan §3.6 item 15: the same patch with and without "extra" intent rows gets the same
    /// level; only the chip differs.
    #[test]
    fn outside_task_never_changes_the_level_only_the_chip() {
        let flags = [flag()];
        let patches: Vec<(Vec<FileDiff>, &[Flag])> = vec![
            // Low: a small change with tests nearby
            (vec![code("src/a.rs", 3), test_edit("tests/a_test.rs")], &[]),
            // Medium, one regular signal away from High: CI plus deploy files
            (vec![modified(".github/workflows/ci.yml", "@@ -1,1 +1,1 @@\n-a\n+b\n"), modified("Dockerfile", "@@ -1,1 +1,1 @@\n-a\n+b\n")], &[]),
            // Medium, one strong signal: a loosened test
            (vec![code("src/a.rs", 3), test_edit("tests/a_test.rs")], &flags),
            // Medium, strong plus regular: one more signal would be High
            (vec![code("src/auth.rs", 3), test_edit("tests/auth_test.rs"), modified(".github/workflows/ci.yml", "@@ -1,1 +1,1 @@\n-a\n+b\n")], &[]),
            // High: a security path and a loosened test (#294's shape)
            (vec![code("src/server/activity/redaction.rs", 3), test_edit("tests/redaction_test.rs")], &flags),
        ];
        let mut levels = Vec::new();
        for (files, tamper) in &patches {
            let without = assess(&Inputs { files, tamper, ..Default::default() });
            let with = assess(&Inputs { files, tamper, outside_task: true, ..Default::default() });
            assert_eq!(with.level, without.level, "{files:?}");
            assert!(with.reasons.iter().any(|r| r == OUTSIDE_TASK) && !without.reasons.iter().any(|r| r == OUTSIDE_TASK));
            let mut rest: Vec<&String> = with.reasons.iter().filter(|r| *r != OUTSIDE_TASK).collect();
            // With nothing else to say, "Small change…" gives way to the chip
            if without.reasons.len() == 2 && without.reasons[0].starts_with("Small change") {
                rest.insert(0, &without.reasons[0]);
            }
            assert_eq!(rest, without.reasons.iter().collect::<Vec<_>>());
            levels.push(without.level);
        }
        assert_eq!(levels, [Level::Low, Level::Medium, Level::Medium, Level::Medium, Level::High]);
    }

    #[test]
    fn levels_combine_strong_and_regular_signals() {
        // #294's shape: security path, a loosened test and a file outside the task → High
        let files = [code("src/server/activity/redaction.rs", 3), test_edit("tests/redaction_test.rs")];
        let flags = [flag()];
        let r = assess(&Inputs { files: &files, tamper: &flags, outside_task: true, ..Default::default() });
        assert_eq!(r.level, Level::High);
        assert_eq!(&r.reasons[..3], ["Security path: redaction", "A test was loosened", OUTSIDE_TASK]);
        // One strong plus one regular is Medium; plus another regular is High
        let mut files = vec![code("src/auth.rs", 3), test_edit("tests/auth_test.rs"), modified(".github/workflows/ci.yml", "@@ -1,1 +1,1 @@\n-a\n+b\n")];
        assert_eq!(risk(&files).level, Level::Medium);
        files.push(modified("Dockerfile", "@@ -1,1 +1,1 @@\n-a\n+b\n"));
        assert_eq!(risk(&files).level, Level::High);
        // Four regular signals are High
        let files = [
            modified(".github/workflows/ci.yml", "@@ -1,1 +1,1 @@\n-a\n+b\n"),
            modified("Dockerfile", "@@ -1,1 +1,1 @@\n-a\n+b\n"),
            modified("frontend/vite.config.ts", "@@ -1,1 +1,1 @@\n-a\n+b\n"),
            code("src/server/reactions.rs", 12),
        ];
        assert_eq!(risk(&files).level, Level::High);
        // A large change is a regular signal (its size chip says why)
        let big: Vec<FileDiff> = (0..16).map(|i| code(&format!("src/m{i}.rs"), 1)).chain([test_edit("tests/m_test.rs")]).collect();
        assert_eq!(risk(&big).level, Level::Medium);
    }

    #[test]
    fn a_secret_or_a_missing_package_is_always_high() {
        let files = [code("src/a.rs", 3), test_edit("tests/a_test.rs")];
        let r = assess(&Inputs { files: &files, secrets: 1, ..Default::default() });
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::High, "Has a secret in the diff"));
        let r = assess(&Inputs { files: &files, missing_packages: 2, ..Default::default() });
        assert_eq!((r.level, r.reasons[0].as_str()), (Level::High, "Adds 2 packages that don't exist"));
    }

    #[test]
    fn the_three_real_merges() {
        let r = |n| {
            let f = fixture(n);
            assess(&Inputs { files: &f.files, first_edit_files: &f.first_edit_files, tamper: &tamper::check(&f.files), ..Default::default() })
        };
        assert_eq!(r(290), Risk { level: Level::Low, reasons: vec!["Small change, tests nearby".into(), "+27 \u{2212}9 in 2 files".into()] });
        assert_eq!(r(301), Risk { level: Level::Low, reasons: vec!["Small change, tests nearby".into(), "+132 \u{2212}13 in 3 files".into()] });
        assert_eq!(r(303), Risk { level: Level::Medium, reasons: vec!["Has a migration".into(), "+126 \u{2212}7 in 4 files".into()] });
    }
}
