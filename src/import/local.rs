//! "Found on this computer" (0.8.0 A2): bots and skills other agent apps keep in the person's
//! home folder: `~/.openclaw`, `~/.hermes`, `~/.claude/agents`, `~/.claude/skills` and `~/.codex`
//! (or `CODEX_HOME`).
//!
//! [`scan`] only lists folders (names, kinds and sizes from `lstat`); the one file it reads is
//! Hermes' `skills/.bundled_manifest`, a list of skill names, so skills Hermes installed itself
//! aren't counted. [`read`] opens only the files each format uses, every one from the source
//! folder's own descriptor with `O_NOFOLLOW` on each step ([`crate::sandbox::safe_fs::Dir`]): a
//! symlink anywhere is skipped and named, never followed. Key stores (`.env`, `auth.json`,
//! `openclaw.json`, `credentials/`, `config.toml`, `state.db`, sessions) are never opened.

use super::archive::{Entry, Unzipped};
use super::parsers::{self, formats};
use super::{Bundle, ImportError, BUNDLE_FORMAT, MAX_FILE_BYTES, MAX_UNZIPPED_BYTES, MAX_ZIP_ENTRIES};
use crate::sandbox::safe_fs::{self, Dir, Kind};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Folders deeper than this below a source aren't looked at.
pub const MAX_DEPTH: usize = 6;
/// At most this many names per source in the list.
pub const MAX_NAMES: usize = 50;
/// At most this many sources ticked at once.
pub const MAX_PICKED: usize = 10;

pub const CLAUDE_AGENTS: &str = "claude-code";
pub const CLAUDE_SKILLS: &str = "skill";

/// One source found on this computer, as the list shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Found {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Shown with `~` for the home folder.
    pub path: String,
    pub bots: usize,
    pub skills: usize,
    /// Memory files (MEMORY.md, USER.md), not entries: entries would need reading.
    pub memories: usize,
    pub names: Vec<String>,
    pub too_large: bool,
}

/// What the files of a source are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    /// Opened and read on import.
    Read,
    /// Only its name counts (a daily log, a first-run script): never opened.
    Seen,
}

/// A source and where it lives.
struct Located {
    found: Found,
    /// The source folder, opened without following a symlink on the way.
    dir: Dir,
    /// Allowlisted files below it: relative path, use, size at listing time.
    files: Vec<(String, Use, u64)>,
    /// Files that are symlinks where a file kwant reads would be.
    links: Vec<String>,
}

fn shown(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => Path::new("~").join(rel).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Whether `rel` (lower case, `/`-separated, no leading slash) is a file `kind` uses.
fn wanted(kind: &str, rel: &str) -> Option<Use> {
    let parts: Vec<&str> = rel.split('/').collect();
    let base = *parts.last()?;
    match (kind, parts.as_slice()) {
        (formats::OPENCLAW, [_]) => match base {
            "soul.md" | "agents.md" | "identity.md" | "user.md" | "memory.md" => Some(Use::Read),
            "boot.md" | "bootstrap.md" => Some(Use::Seen),
            _ => None,
        },
        (formats::OPENCLAW, ["memory", f]) if f.ends_with(".md") => Some(Use::Seen),
        (formats::OPENCLAW | formats::CODEX, ["skills", _, "skill.md"]) => Some(Use::Read),
        (formats::HERMES, ["soul.md"] | ["memories", "memory.md" | "user.md"] | ["skills", ".bundled_manifest"]) => Some(Use::Read),
        (formats::HERMES, ["skills", .., "skill.md"]) if parts.len() >= 3 => Some(Use::Read),
        (formats::CODEX, ["agents.md" | "agents.override.md"]) => Some(Use::Read),
        (CLAUDE_AGENTS, [.., f]) if parts.len() <= 2 && f.ends_with(".md") => Some(Use::Read),
        (CLAUDE_SKILLS, [_, "skill.md"]) => Some(Use::Read),
        _ => None,
    }
}

/// Whether to look inside the folder `rel` of a `kind` source.
fn descend(kind: &str, rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return false;
    }
    match kind {
        formats::OPENCLAW => matches!(parts.as_slice(), ["skills"] | ["skills", _] | ["memory"]),
        formats::CODEX => matches!(parts.as_slice(), ["skills"] | ["skills", _]),
        // Profiles are sources of their own.
        formats::HERMES => matches!(parts.first(), Some(&"memories") | Some(&"skills")),
        CLAUDE_AGENTS => parts.len() == 1,
        CLAUDE_SKILLS => parts.len() == 1,
        _ => false,
    }
}

/// Lists the files a `kind` source uses: names and sizes only.
fn walk(dir: &Dir, kind: &str, prefix: &str, depth: usize, files: &mut Vec<(String, Use, u64)>, links: &mut Vec<String>) {
    let Ok(mut entries) = dir.list() else { return };
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in entries {
        let Some(name) = e.name.to_str() else { continue };
        let rel = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        let lower = rel.to_lowercase();
        match e.kind {
            Kind::File => {
                if let Some(u) = wanted(kind, &lower) {
                    files.push((rel, u, e.len));
                }
            }
            Kind::Dir if depth < MAX_DEPTH && descend(kind, &lower) => {
                if let Ok(sub) = dir.open_dir(Path::new(name)) {
                    walk(&sub, kind, &rel, depth + 1, files, links);
                }
            }
            Kind::Symlink if wanted(kind, &lower).is_some() || descend(kind, &lower) => links.push(rel),
            _ => {}
        }
    }
}

fn skill_folder(rel: &str) -> Option<String> {
    let (dir, base) = rel.rsplit_once('/')?;
    base.eq_ignore_ascii_case("skill.md").then(|| dir.rsplit('/').next().unwrap_or(dir).to_string())
}

/// Skill names Hermes seeded itself (`skills/.bundled_manifest`), read from the source's handle.
fn hermes_bundled(dir: &Dir, files: &[(String, Use, u64)]) -> BTreeSet<String> {
    let Some((rel, _, _)) = files.iter().find(|(r, _, _)| r.eq_ignore_ascii_case("skills/.bundled_manifest")) else { return BTreeSet::new() };
    let Ok(text) = dir.open_file(Path::new(rel)).and_then(|f| safe_fs::read_capped(f, 256 * 1024)) else { return BTreeSet::new() };
    String::from_utf8_lossy(&text).lines().filter_map(|l| l.split(':').next()).map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect()
}

fn locate(dir: Dir, id: String, kind: &str, label: String, path: String) -> Option<Located> {
    let (mut files, mut links) = (Vec::new(), Vec::new());
    walk(&dir, kind, "", 0, &mut files, &mut links);
    let bundled = if kind == formats::HERMES { hermes_bundled(&dir, &files) } else { BTreeSet::new() };
    let mut names = BTreeSet::new();
    let mut skills = 0;
    let mut bots = 0;
    let mut memories = 0;
    for (rel, u, _) in &files {
        if *u != Use::Read {
            continue;
        }
        let lower = rel.to_lowercase();
        if let Some(folder) = skill_folder(rel) {
            if kind == formats::HERMES && bundled.contains(&folder.to_lowercase()) {
                continue;
            }
            skills += 1;
            names.insert(folder);
        } else if kind == CLAUDE_AGENTS {
            bots += 1;
            names.insert(rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".md").to_string());
        } else if matches!(lower.as_str(), "memory.md" | "user.md" | "memories/memory.md" | "memories/user.md") {
            memories += 1;
        }
    }
    let read: Vec<&(String, Use, u64)> = files.iter().filter(|(r, u, _)| *u == Use::Read && !r.ends_with(".bundled_manifest")).collect();
    let has_bot_file = read.iter().any(|(r, _, _)| skill_folder(r).is_none());
    if matches!(kind, formats::OPENCLAW | formats::HERMES | formats::CODEX) && has_bot_file {
        bots = 1;
    }
    if bots == 0 && skills == 0 {
        return None;
    }
    let total: u64 = read.iter().map(|(_, _, n)| n).sum();
    let found = Found {
        id,
        kind: kind.to_string(),
        label,
        path,
        bots,
        skills,
        memories,
        names: names.into_iter().take(MAX_NAMES).collect(),
        too_large: read.len() > MAX_ZIP_ENTRIES || total > MAX_UNZIPPED_BYTES,
    };
    Some(Located { found, dir, files, links })
}

/// A name that is safe in an id: letters, digits, `-`, `_` and `.`, at most 64.
fn plain_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Subfolders of `dir` (real folders only) whose names start with `prefix`, with the rest of the name.
fn subdirs<'a>(dir: &'a Dir, prefix: &'a str) -> impl Iterator<Item = (String, String)> + 'a {
    let mut list = dir.list().unwrap_or_default();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list.into_iter().filter(|e| e.kind == Kind::Dir).filter_map(move |e| {
        let name = e.name.to_str()?.to_string();
        let rest = name.strip_prefix(prefix)?.to_string();
        Some((name, rest))
    })
}

fn find_all(home: &Path, codex_home: Option<&Path>) -> Vec<Located> {
    let Ok(home_dir) = Dir::open(home) else { return Vec::new() };
    let mut out = Vec::new();
    let mut add = |dir: std::io::Result<Dir>, id: String, kind: &str, label: String, path: PathBuf| {
        if let Ok(d) = dir {
            if let Some(l) = locate(d, id, kind, label, shown(home, &path)) {
                out.push(l);
            }
        }
    };
    // OpenClaw: ~/.openclaw/workspace, extra agents in workspace-<agentId>, profiles in ~/.openclaw-<profile>
    if let Ok(oc) = home_dir.open_dir(Path::new(".openclaw")) {
        add(oc.open_dir(Path::new("workspace")), "openclaw".into(), formats::OPENCLAW, "OpenClaw".into(), home.join(".openclaw/workspace"));
        for (name, agent) in subdirs(&oc, "workspace-").collect::<Vec<_>>() {
            if plain_name(&agent) {
                let label = format!("OpenClaw ({agent})");
                add(oc.open_dir(Path::new(&name)), format!("openclaw:{agent}"), formats::OPENCLAW, label, home.join(".openclaw").join(&name));
            }
        }
    }
    for (name, profile) in subdirs(&home_dir, ".openclaw-").collect::<Vec<_>>() {
        if plain_name(&profile) {
            let dir = home_dir.open_dir(&Path::new(&name).join("workspace"));
            add(dir, format!("openclaw-{profile}"), formats::OPENCLAW, format!("OpenClaw ({profile} profile)"), home.join(&name).join("workspace"));
        }
    }
    // Hermes: ~/.hermes and ~/.hermes/profiles/<name>
    if let Ok(hm) = home_dir.open_dir(Path::new(".hermes")) {
        let profiles = hm.open_dir(Path::new("profiles")).ok();
        add(Ok(hm), "hermes".into(), formats::HERMES, "Hermes Agent".into(), home.join(".hermes"));
        if let Some(p) = profiles {
            for (name, _) in subdirs(&p, "").collect::<Vec<_>>() {
                if plain_name(&name) {
                    let label = format!("Hermes Agent ({name} profile)");
                    add(p.open_dir(Path::new(&name)), format!("hermes:{name}"), formats::HERMES, label, home.join(".hermes/profiles").join(&name));
                }
            }
        }
    }
    add(home_dir.open_dir(Path::new(".claude/agents")), "claude-agents".into(), CLAUDE_AGENTS, "Claude Code agents".into(), home.join(".claude/agents"));
    add(home_dir.open_dir(Path::new(".claude/skills")), "claude-skills".into(), CLAUDE_SKILLS, "Claude Code skills".into(), home.join(".claude/skills"));
    // Codex: CODEX_HOME when set (refused when any part of it is a symlink the person doesn't own as root), else ~/.codex
    match codex_home.filter(|p| p.is_absolute()) {
        Some(p) => add(Dir::open_strict(p), "codex".into(), formats::CODEX, "Codex".into(), p.to_path_buf()),
        None => add(home_dir.open_dir(Path::new(".codex")), "codex".into(), formats::CODEX, "Codex".into(), home.join(".codex")),
    }
    out
}

/// What other agent apps keep in `home`. Lists folders only (see the module notes).
pub fn scan(home: &Path, codex_home: Option<&Path>) -> Vec<Found> {
    find_all(home, codex_home).into_iter().map(|l| l.found).collect()
}

/// The person's home folder and `CODEX_HOME`, as the app sees them.
pub fn home() -> Option<(PathBuf, Option<PathBuf>)> {
    let home = dirs::home_dir()?;
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    Some((home, codex))
}

#[derive(Debug, PartialEq)]
pub enum ReadError {
    /// An id that isn't on this computer (any longer).
    NotFound(String),
    Import(ImportError),
}

/// Reads the ticked sources into one bundle. Ids are the ones [`scan`] gave.
pub fn read(home: &Path, codex_home: Option<&Path>, ids: &[String]) -> Result<Bundle, ReadError> {
    let mut all = find_all(home, codex_home);
    let mut picked = Vec::new();
    for id in ids {
        let Some(i) = all.iter().position(|l| &l.found.id == id) else { return Err(ReadError::NotFound(id.clone())) };
        picked.push(all.swap_remove(i));
    }
    let too_large = |m: String| ReadError::Import(ImportError::TooLarge(m));
    let (mut count, mut total) = (0usize, 0u64);
    let mut merged = Bundle { format: BUNDLE_FORMAT.into(), ..Bundle::default() };
    let mut kinds = BTreeSet::new();
    let mut labels = Vec::new();
    for l in picked {
        let mut entries = Vec::new();
        let mut seen = Vec::new();
        let mut skipped = l.links.clone();
        for (rel, u, _) in &l.files {
            if *u == Use::Seen {
                seen.push(rel.clone());
                continue;
            }
            count += 1;
            if count > MAX_ZIP_ENTRIES {
                return Err(too_large(format!("These folders have more than {MAX_ZIP_ENTRIES} files kwant would read. Zip the parts you want instead.")));
            }
            match l.dir.open_file(Path::new(rel)).and_then(|f| safe_fs::read_capped(f, MAX_FILE_BYTES as u64)) {
                Ok(bytes) => {
                    total += bytes.len() as u64;
                    if total > MAX_UNZIPPED_BYTES {
                        return Err(too_large("These folders hold more than 50 MB kwant would read. Zip the parts you want instead.".into()));
                    }
                    entries.push(Entry { path: rel.clone(), bytes });
                }
                Err(e) if safe_fs::is_too_large(&e) => return Err(too_large(format!("{}/{rel} is larger than 4 MB.", l.found.path))),
                // A symlink, a FIFO or a file gone since the listing: skipped and named.
                Err(_) => skipped.push(rel.clone()),
            }
        }
        let mut b = match l.found.kind.as_str() {
            k @ (formats::OPENCLAW | formats::HERMES | formats::CODEX) => {
                seen.extend(skipped.iter().cloned());
                let tree = formats::Tree::new(&entries, &seen);
                let mut b = formats::parse_kind(k, &tree).unwrap_or_default();
                name_after_source(&mut b, &l.found);
                b
            }
            _ => parsers::from_entries(Unzipped { entries, skipped: Vec::new(), comment: None }, true).map_err(ReadError::Import)?,
        };
        if !skipped.is_empty() {
            let shown: Vec<&str> = skipped.iter().take(10).map(String::as_str).collect();
            b.omitted.push(super::cut(&format!("Skipped in {} (links or unreadable): {}", l.found.path, shown.join(", ")), 200));
        }
        kinds.insert(l.found.kind.clone());
        labels.push(l.found.label.clone());
        merged.bots.extend(b.bots);
        merged.teams.extend(b.teams);
        merged.skills.extend(b.skills);
        merged.omitted.extend(b.omitted);
    }
    if merged.bots.is_empty() && merged.skills.is_empty() {
        return Err(ReadError::Import(ImportError::Unsupported("There are no bots or skills kwant can read here.".into())));
    }
    merged.source.kind = if kinds.len() == 1 { kinds.into_iter().next().unwrap_or_default() } else { "local".into() };
    merged.source.label = Some(format!("Found on this computer: {}", labels.join(", ")));
    Ok(merged)
}

/// A profile or extra agent is named after its folder unless its own files name it.
fn name_after_source(b: &mut Bundle, found: &Found) {
    let Some(bot) = b.bots.first_mut() else { return };
    let given = match found.id.split_once([':', '-']) {
        Some((_, n)) if !n.is_empty() => n.to_string(),
        _ => return,
    };
    if bot.name == "OpenClaw" || bot.name == "Hermes" {
        bot.ref_ = super::slug(&given);
        bot.name = given;
        if found.kind == formats::HERMES {
            bot.title = "Hermes Agent profile".into();
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
