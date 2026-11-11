//! Business packs (`schema_version` 1): a ready-made team as a folder anyone can write, check
//! and share.
//!
//! ```text
//! packs/<id>/pack.json     the team: bots, needs, routines, asks (see docs/teams-api.md)
//! packs/<id>/skills/*.md   one skill per file: front matter `name`, `description`, `owner`
//! packs/<id>/README.md     what the team does, for people
//! ```
//!
//! [`check_files`] reads a pack and refuses anything a pack may not do: carry a secret, add
//! tools, MCP servers or approvals, turn on Dev mode, ship a routine switched on, name an
//! unknown preset, or use a `{{placeholder}}` it doesn't declare in `asks`. The built-in packs
//! (`teams::templates`), `kwnt pack check` and the importer all go through it, so a pack that
//! passes the check is one the create can take as it is.

use super::templates::{Need, Routine, Template, TemplateBot, TemplateSkill, MAX_SCOPE_CHARS, MAX_SCOPE_ITEMS, MAX_SKILL_BODY_CHARS, MAX_SKILL_DESCRIPTION_CHARS, MAX_SKILL_NAME_CHARS};
use super::{names, presets};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// The pack format this build reads.
pub const SCHEMA_VERSION: u64 = 1;
/// Bots in one pack (the create's own limit).
pub const MAX_BOTS: usize = 6;
pub const MAX_ROUTINES: usize = 10;
pub const MAX_ASKS: usize = 10;
/// One answer to an ask, in characters.
pub const MAX_ANSWER_CHARS: usize = 1000;
/// Persona, soul and instructions, in characters (the importer's cap).
pub const MAX_TEXT_CHARS: usize = crate::import::MAX_PERSONA_CHARS;
/// One file in a pack, in bytes, and files in a pack.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_FILES: usize = 64;
const MAX_ASK_ID_CHARS: usize = 40;

/// What a need's button may open. A pack can point the person at these; it can't add new ones.
pub const KNOWN_OPENS: [&str; 4] = ["loose_ends_imap", "connections", "composer_prompt", "dev_mode_setup"];

/// One question the pack asks when the team is made; text uses it as `{{id}}`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub example: String,
}

/// `pack.json` as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFile {
    /// Checked on the raw JSON first, so a newer pack gets a plain message.
    #[allow(dead_code)]
    schema_version: u64,
    id: String,
    version: String,
    field: String,
    label: String,
    icon: String,
    handle: String,
    card_line: String,
    mission: String,
    instructions: String,
    #[serde(default)]
    asks: Vec<Ask>,
    #[serde(default)]
    needs: Vec<Need>,
    #[serde(default)]
    never: Vec<String>,
    /// The 0.9 single routine, still read; a pack sets this or `routines`, not both.
    #[serde(default)]
    routine: Option<Routine>,
    #[serde(default)]
    routines: Option<Vec<Routine>>,
    #[serde(default)]
    first_prompts: Vec<String>,
    bots: Vec<TemplateBot>,
}

/// Keys that would widen what a bot may do. A pack only describes a team; tools, MCP servers,
/// approvals and Dev mode stay the person's to turn on.
const FORBIDDEN_KEYS: [(&str, &str); 14] = [
    ("tools", "adds tools"),
    ("allowed_tools", "adds tools"),
    ("allowed-tools", "adds tools"),
    ("allowedtools", "adds tools"),
    ("mcp", "adds MCP servers"),
    ("mcp_servers", "adds MCP servers"),
    ("mcpservers", "adds MCP servers"),
    ("approvals", "changes approvals"),
    ("approval", "changes approvals"),
    ("approval_mode", "changes approvals"),
    ("permissions", "changes approvals"),
    ("execution_mode", "changes approvals"),
    ("dev_mode", "turns on Dev mode"),
    ("devmode", "turns on Dev mode"),
];

fn forbidden(key: &str) -> Option<&'static str> {
    let k = key.to_ascii_lowercase();
    FORBIDDEN_KEYS.iter().find(|(f, _)| *f == k).map(|(_, why)| *why)
}

/// Reads a pack folder from disk and checks it. Hidden entries (`.git`, `.DS_Store`) are
/// skipped; links are refused.
pub fn check_dir(dir: &Path) -> Result<Template, Vec<String>> {
    let mut files = BTreeMap::new();
    let mut problems = Vec::new();
    if !dir.is_dir() {
        return Err(vec![format!("{} is not a folder", dir.display())]);
    }
    read_dir_into(dir, dir, &mut files, &mut problems);
    if !problems.is_empty() {
        return Err(problems);
    }
    check_files(&files)
}

fn read_dir_into(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>, problems: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return problems.push(format!("can't read {}: {e}", dir.display())),
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let rel = path.strip_prefix(root).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).unwrap_or(name);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("can't read {rel}: {e}"));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            problems.push(format!("{rel} is a link; a pack holds only plain files"));
        } else if meta.is_dir() {
            read_dir_into(root, &path, files, problems);
        } else if files.len() >= MAX_FILES {
            problems.push(format!("more than {MAX_FILES} files in the pack"));
            return;
        } else if meta.len() as usize > MAX_FILE_BYTES {
            problems.push(format!("{rel} is over {} KB", MAX_FILE_BYTES / 1024));
        } else {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    files.insert(rel, bytes);
                }
                Err(e) => problems.push(format!("can't read {rel}: {e}")),
            }
        }
    }
}

/// What `kwnt pack check <dir>` prints, one line each, and whether the pack passed.
pub fn check_report(dir: &Path) -> (Vec<String>, bool) {
    match check_dir(dir) {
        Ok(t) => {
            let routines = t.routines.len();
            let line = format!(
                "ok: {} {} ({}): {} bots, {} skills, {} routine{}{}, {} ask{}",
                t.id,
                t.version,
                t.field,
                t.bots.len(),
                t.skills.len(),
                routines,
                if routines == 1 { "" } else { "s" },
                if routines == 0 { "" } else { " (off)" },
                t.asks.len(),
                if t.asks.len() == 1 { "" } else { "s" }
            );
            (vec![line], true)
        }
        Err(problems) => {
            let mut lines: Vec<String> = problems.into_iter().map(|p| format!("problem: {p}")).collect();
            lines.push(format!("{} problem{}; the pack can't be used until they are fixed", lines.len(), if lines.len() == 1 { "" } else { "s" }));
            (lines, false)
        }
    }
}

/// Whether these files (paths relative to the pack folder) look like a pack: a `pack.json`
/// with a `schema_version`. The importer uses this to send a folder here.
pub fn looks_like_pack(pack_json: &[u8]) -> bool {
    serde_json::from_slice::<Value>(pack_json).is_ok_and(|v| v.get("schema_version").is_some() && v.get("bots").is_some())
}

/// Checks a pack given as its files (path in the pack -> bytes) and returns it as a template.
/// Every problem is one plain line; the pack is refused if there is any.
pub fn check_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Template, Vec<String>> {
    let mut p = Vec::new();
    let mut skill_files = Vec::new();
    let mut readme = None;
    let mut pack_json = None;
    for (path, bytes) in files {
        if path.split('/').any(|seg| seg.starts_with('.')) {
            continue;
        }
        if bytes.len() > MAX_FILE_BYTES {
            p.push(format!("{path} is over {} KB", MAX_FILE_BYTES / 1024));
            continue;
        }
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                p.push(format!("{path} is not UTF-8 text"));
                continue;
            }
        };
        let (_, secrets) = crate::import::scrub::scrub_text(text);
        if secrets > 0 && path != "pack.json" {
            p.push(format!("{path} has what looks like a key, token or password; take it out"));
        }
        match path.as_str() {
            "pack.json" => pack_json = Some(text),
            "README.md" => readme = Some(text),
            _ if path.starts_with("skills/") && path.ends_with(".md") && path.matches('/').count() == 1 => skill_files.push((path.as_str(), text)),
            _ if !path.contains('/') && path.to_ascii_uppercase().starts_with("LICENSE") => {}
            _ => p.push(format!("{path} doesn't belong in a pack; a pack holds pack.json, README.md and skills/*.md")),
        }
    }
    let Some(pack_json) = pack_json else {
        p.push("pack.json is missing".into());
        return Err(p);
    };
    let readme = match readme {
        Some(r) if !r.trim().is_empty() => r.to_string(),
        _ => {
            p.push("README.md is missing or empty; say in it what the team does".into());
            String::new()
        }
    };

    let mut raw: Value = match serde_json::from_str(pack_json) {
        Ok(v) => v,
        Err(e) => {
            p.push(format!("pack.json is not valid JSON: {e}"));
            return Err(p);
        }
    };
    if !raw.is_object() {
        p.push("pack.json must be a JSON object".into());
        return Err(p);
    }
    strip_forbidden(&mut raw, "", &mut p);
    secrets_in(&raw, "", &mut p);
    match raw.get("schema_version") {
        Some(Value::Number(n)) if n.as_u64() == Some(SCHEMA_VERSION) => {}
        Some(v) => {
            p.push(format!("pack.json: schema_version is {v}; this kwant reads schema_version {SCHEMA_VERSION}"));
            return Err(p);
        }
        None => {
            p.push(format!("pack.json: schema_version is missing; set it to {SCHEMA_VERSION}"));
            return Err(p);
        }
    }
    if raw.get("skills").is_some() {
        p.push("pack.json: skills go in skills/*.md, not in pack.json".into());
        if let Value::Object(m) = &mut raw {
            m.remove("skills");
        }
    }
    let pack: PackFile = match serde_json::from_value(raw.clone()) {
        Ok(f) => f,
        Err(e) => {
            p.push(format!("pack.json: {e}"));
            return Err(p);
        }
    };

    // The team.
    if !valid_id(&pack.id) {
        p.push(format!("id {:?} must be a slug: a-z, 0-9 and -", pack.id));
    }
    for (name, v) in [("version", &pack.version), ("field", &pack.field), ("label", &pack.label), ("icon", &pack.icon), ("card_line", &pack.card_line), ("mission", &pack.mission), ("instructions", &pack.instructions)] {
        if v.trim().is_empty() {
            p.push(format!("{name} is empty"));
        }
    }
    if !names::valid_handle(&pack.handle) {
        p.push(format!("handle {:?} must be a slug: a-z, 0-9 and -, at most 40 characters", pack.handle));
    }
    if pack.instructions.chars().count() > MAX_TEXT_CHARS {
        p.push(format!("instructions are over {MAX_TEXT_CHARS} characters"));
    }

    // Bots.
    if pack.bots.is_empty() || pack.bots.len() > MAX_BOTS {
        p.push(format!("a pack has 1 to {MAX_BOTS} bots; this one has {}", pack.bots.len()));
    }
    let leads = pack.bots.iter().filter(|b| b.lead).count();
    if !pack.bots.is_empty() && leads != 1 {
        p.push(format!("exactly one bot must be the lead; {leads} are"));
    }
    let mut slots = HashSet::new();
    for b in &pack.bots {
        if !names::valid_name(&b.name) {
            p.push(format!("bot name {:?} must be one word, at most 40 characters", b.name));
        }
        if !slots.insert(b.name.to_lowercase()) {
            p.push(format!("two bots are called {}", b.name));
        }
        if presets::get(&b.preset).is_none() {
            p.push(format!("bot {}: unknown preset {:?}", b.name, b.preset));
        }
        for (what, text) in [("persona", &b.persona), ("soul", &b.soul)] {
            if text.trim().is_empty() {
                p.push(format!("bot {}: {what} is empty", b.name));
            } else if text.chars().count() > MAX_TEXT_CHARS {
                p.push(format!("bot {}: {what} is over {MAX_TEXT_CHARS} characters", b.name));
            }
        }
        for (what, list) in [("good_for", &b.good_for), ("not_for", &b.not_for)] {
            if list.len() > MAX_SCOPE_ITEMS {
                p.push(format!("bot {}: {what} has more than {MAX_SCOPE_ITEMS} items", b.name));
            }
            if list.iter().any(|s| s.trim().is_empty() || s.chars().count() > MAX_SCOPE_CHARS) {
                p.push(format!("bot {}: each {what} item is 1 to {MAX_SCOPE_CHARS} characters", b.name));
            }
        }
    }
    let has_bot = |slot: &str| pack.bots.iter().any(|b| b.name == slot);

    // Needs.
    let mut need_ids = HashSet::new();
    for n in &pack.needs {
        if !need_ids.insert(n.id.as_str()) {
            p.push(format!("two needs have the id {}", n.id));
        }
        if !KNOWN_OPENS.contains(&n.opens.as_str()) {
            p.push(format!("need {}: opens {:?} is not one of {}", n.id, n.opens, KNOWN_OPENS.join(", ")));
        }
    }

    // Routines: always paused, a valid cron, run by a bot of the pack.
    let routines = match (pack.routine, pack.routines) {
        (Some(_), Some(_)) => {
            p.push("set routines or routine, not both".into());
            Vec::new()
        }
        (Some(r), None) => vec![r],
        (None, Some(rs)) => rs,
        (None, None) => Vec::new(),
    };
    if routines.len() > MAX_ROUTINES {
        p.push(format!("more than {MAX_ROUTINES} routines"));
    }
    for r in &routines {
        if r.enabled {
            p.push(format!("routine {:?} is switched on; a pack's routines start off, and the person turns them on", r.name));
        }
        let schedule = crate::routines::Schedule::Cron { expression: r.cron.clone(), tz: "UTC".into() };
        if let Err(e) = schedule.validate() {
            p.push(format!("routine {:?}: bad cron {:?}: {e}", r.name, r.cron));
        }
        if !has_bot(&r.bot) {
            p.push(format!("routine {:?} is run by {:?}, which isn't a bot in this pack", r.name, r.bot));
        }
        if r.name.trim().is_empty() || r.name.chars().count() > crate::import::MAX_ROUTINE_NAME_CHARS {
            p.push(format!("routine {:?}: the name is 1 to {} characters", r.name, crate::import::MAX_ROUTINE_NAME_CHARS));
        }
        if r.prompt.trim().is_empty() {
            p.push(format!("routine {:?}: prompt is empty", r.name));
        }
    }

    // Skills.
    let mut skills: Vec<(usize, TemplateSkill)> = Vec::new();
    let mut skill_names = HashSet::new();
    for (path, text) in skill_files {
        match parse_skill(text) {
            Err(e) => p.push(format!("{path}: {e}")),
            Ok((meta_problems, skill)) => {
                p.extend(meta_problems.into_iter().map(|e| format!("{path}: {e}")));
                let Some(skill) = skill else { continue };
                let n = skill.name.chars().count();
                if !(1..=MAX_SKILL_NAME_CHARS).contains(&n) {
                    p.push(format!("{path}: the name is 1 to {MAX_SKILL_NAME_CHARS} characters"));
                }
                if skill.description.trim().is_empty() || skill.description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
                    p.push(format!("{path}: the description is 1 to {MAX_SKILL_DESCRIPTION_CHARS} characters"));
                }
                if skill.body.trim().is_empty() {
                    p.push(format!("{path}: the skill has no text"));
                } else if skill.body.chars().count() > MAX_SKILL_BODY_CHARS {
                    p.push(format!("{path}: the skill is {} characters; the limit is {MAX_SKILL_BODY_CHARS}", skill.body.chars().count()));
                }
                if !skill_names.insert(skill.name.to_lowercase()) {
                    p.push(format!("{path}: another skill is also called {:?}", skill.name));
                }
                match pack.bots.iter().position(|b| b.name == skill.owner) {
                    Some(i) => skills.push((i, skill)),
                    None => p.push(format!("{path}: owner {:?} isn't a bot in this pack", skill.owner)),
                }
            }
        }
    }
    // Bot order, then file name (the files come sorted).
    skills.sort_by_key(|(i, _)| *i);
    let skills: Vec<TemplateSkill> = skills.into_iter().map(|(_, s)| s).collect();

    // Asks and placeholders.
    let mut ask_ids = HashSet::new();
    if pack.asks.len() > MAX_ASKS {
        p.push(format!("more than {MAX_ASKS} asks"));
    }
    for a in &pack.asks {
        if !valid_ask_id(&a.id) {
            p.push(format!("ask id {:?} must be a-z, 0-9 and _, at most {MAX_ASK_ID_CHARS} characters", a.id));
        }
        if !ask_ids.insert(a.id.as_str()) {
            p.push(format!("two asks have the id {}", a.id));
        }
        if a.label.trim().is_empty() {
            p.push(format!("ask {}: label is empty", a.id));
        }
    }
    let mut used = HashSet::new();
    let mut filled: Vec<(String, &str)> = vec![("instructions".into(), pack.instructions.as_str())];
    for b in &pack.bots {
        filled.push((format!("bot {}: persona", b.name), &b.persona));
        filled.push((format!("bot {}: soul", b.name), &b.soul));
    }
    for s in &skills {
        filled.push((format!("skill {:?}", s.name), &s.body));
    }
    for r in &routines {
        filled.push((format!("routine {:?}: prompt", r.name), &r.prompt));
    }
    for (where_, text) in &filled {
        match placeholders(text) {
            Err(bad) => p.push(format!("{where_}: malformed placeholder {bad:?}; write {{{{id}}}} with a-z, 0-9 and _")),
            Ok(ids) => {
                for id in ids {
                    if !ask_ids.contains(id.as_str()) {
                        p.push(format!("{where_}: {{{{{id}}}}} isn't declared in asks"));
                    }
                    used.insert(id);
                }
            }
        }
    }
    // Anywhere else, a placeholder would reach the person unfilled.
    let mut elsewhere = Vec::new();
    if let Value::Object(m) = &raw {
        for (k, v) in m {
            match k.as_str() {
                "instructions" => {}
                "bots" => {
                    for (i, b) in v.as_array().into_iter().flatten().enumerate() {
                        for (bk, bv) in b.as_object().into_iter().flatten() {
                            if bk != "persona" && bk != "soul" {
                                strings_with_braces(bv, &format!("bots[{i}].{bk}"), &mut elsewhere);
                            }
                        }
                    }
                }
                "routine" | "routines" => {
                    let list: Vec<&Value> = if k == "routine" { vec![v] } else { v.as_array().into_iter().flatten().collect() };
                    for (i, r) in list.into_iter().enumerate() {
                        for (rk, rv) in r.as_object().into_iter().flatten() {
                            if rk != "prompt" {
                                strings_with_braces(rv, &format!("{k}[{i}].{rk}"), &mut elsewhere);
                            }
                        }
                    }
                }
                _ => strings_with_braces(v, k, &mut elsewhere),
            }
        }
    }
    for s in &skills {
        if s.name.contains("{{") || s.description.contains("{{") {
            elsewhere.push(format!("skill {:?} name or description", s.name));
        }
    }
    for at in elsewhere {
        p.push(format!("{at}: placeholders only work in instructions, persona, soul, skill text and routine prompts"));
    }
    for a in &pack.asks {
        if valid_ask_id(&a.id) && !used.contains(&a.id) {
            p.push(format!("ask {} isn't used; put {{{{{}}}}} in the text that needs it", a.id, a.id));
        }
    }

    if !p.is_empty() {
        return Err(p);
    }
    Ok(Template {
        id: pack.id,
        version: pack.version,
        field: pack.field,
        label: pack.label,
        icon: pack.icon,
        handle: pack.handle,
        card_line: pack.card_line,
        mission: pack.mission,
        instructions: pack.instructions,
        asks: pack.asks,
        needs: pack.needs,
        never: pack.never,
        routine: routines.first().cloned(),
        routines,
        first_prompts: pack.first_prompts,
        bots: pack.bots,
        skills,
        readme,
    })
}

/// Removes forbidden keys at any depth, with one line for each.
fn strip_forbidden(v: &mut Value, at: &str, p: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                if let Some(why) = forbidden(&k) {
                    p.push(format!("pack.json: {path} {why}; a pack can't do that, the person sets it up themselves"));
                    m.remove(&k);
                } else if let Some(child) = m.get_mut(&k) {
                    strip_forbidden(child, &path, p);
                }
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter_mut().enumerate() {
                strip_forbidden(child, &format!("{at}[{i}]"), p);
            }
        }
        _ => {}
    }
}

/// One line for each string in pack.json that holds what looks like a secret.
fn secrets_in(v: &Value, at: &str, p: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if crate::import::scrub::scrub_text(s).1 > 0 {
                p.push(format!("pack.json: {at} has what looks like a key, token or password; take it out"));
            }
        }
        Value::Object(m) => {
            for (k, child) in m {
                let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                if crate::import::scrub::scrub_text(k).1 > 0 {
                    p.push(format!("pack.json: the key {path:?} looks like a key, token or password; take it out"));
                }
                secrets_in(child, &path, p);
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate() {
                secrets_in(child, &format!("{at}[{i}]"), p);
            }
        }
        _ => {}
    }
}

fn strings_with_braces(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.contains("{{") => out.push(at.to_string()),
        Value::Object(m) => m.iter().for_each(|(k, c)| strings_with_braces(c, &format!("{at}.{k}"), out)),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, c)| strings_with_braces(c, &format!("{at}[{i}]"), out)),
        _ => {}
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !id.starts_with('-')
}

pub fn valid_ask_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ASK_ID_CHARS && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The `{{id}}` placeholders in `text`, in order (with repeats). `Err` holds the first `{{`
/// that isn't a well-formed placeholder.
pub fn placeholders(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        let Some(end) = after.find("}}") else {
            return Err(rest[i..].chars().take(20).collect());
        };
        let id = &after[..end];
        if !valid_ask_id(id) {
            return Err(rest[i..i + 2 + end + 2].chars().take(40).collect());
        }
        out.push(id.to_string());
        rest = &after[end + 2..];
    }
    Ok(out)
}

/// `text` with each `{{id}}` replaced by its answer. A line holding a placeholder without an
/// answer is left out whole, so a team made without answers reads as if the line was never
/// there. Unknown placeholders count as unanswered (the check refuses them in a pack).
pub fn fill(text: &str, answers: &HashMap<String, String>) -> String {
    if !text.contains("{{") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let Ok(ids) = placeholders(line) else {
            out.push_str(line);
            continue;
        };
        if ids.is_empty() {
            out.push_str(line);
            continue;
        }
        if ids.iter().any(|id| answers.get(id).is_none_or(|a| a.is_empty())) {
            continue;
        }
        let mut l = line.to_string();
        for id in ids {
            l = l.replace(&format!("{{{{{id}}}}}"), &answers[&id]);
        }
        out.push_str(&l);
    }
    out
}

/// Cleans one answer: hidden characters (except new lines) out, trimmed. `None` if it is too
/// long.
pub fn clean_answer(value: &str) -> Option<String> {
    let v: String = value.chars().filter(|&c| c == '\n' || !crate::import::is_hidden(c)).collect();
    let v = v.trim().to_string();
    (v.chars().count() <= MAX_ANSWER_CHARS).then_some(v)
}

impl Template {
    /// This template with the person's answers filled in (instructions, personas, souls, skill
    /// text and routine prompts). With no answers, lines that need one are left out.
    pub fn filled(&self, answers: &HashMap<String, String>) -> Template {
        let mut t = self.clone();
        t.instructions = fill(&t.instructions, answers);
        for b in &mut t.bots {
            b.persona = fill(&b.persona, answers);
            b.soul = fill(&b.soul, answers);
        }
        for s in &mut t.skills {
            s.body = fill(&s.body, answers);
        }
        for r in t.routines.iter_mut().chain(t.routine.iter_mut()) {
            r.prompt = fill(&r.prompt, answers);
        }
        t
    }
}

/// A skill file: `---`, `name:`, `description:`, `owner:` (plain or JSON-quoted), `---`, then
/// the skill text. Returns the problems with the front matter, and the skill when it has all
/// three fields.
fn parse_skill(text: &str) -> Result<(Vec<String>, Option<TemplateSkill>), String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let rest = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")).ok_or("must start with a --- line, then name, description and owner")?;
    let mut fields: Map<String, Value> = Map::new();
    let mut problems = Vec::new();
    let mut body = None;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let l = line.trim_end_matches(['\n', '\r']);
        if l == "---" {
            body = Some(&rest[offset..]);
            break;
        }
        if l.trim().is_empty() {
            continue;
        }
        let Some((k, v)) = l.split_once(':') else {
            problems.push(format!("front matter line {l:?} isn't key: value"));
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        let v = if v.starts_with('"') {
            match serde_json::from_str::<String>(v) {
                Ok(s) => s,
                Err(_) => {
                    problems.push(format!("{k}: the quoted value isn't closed properly"));
                    continue;
                }
            }
        } else {
            v.to_string()
        };
        if let Some(why) = forbidden(k) {
            problems.push(format!("{k} {why}; a pack can't do that, the person sets it up themselves"));
        } else if !matches!(k, "name" | "description" | "owner") {
            problems.push(format!("unknown front matter key {k:?}; a skill has name, description and owner"));
        } else if fields.insert(k.to_string(), Value::String(v)).is_some() {
            problems.push(format!("{k} is set twice"));
        }
    }
    let body = body.ok_or("the front matter has no closing --- line")?;
    let body = body.replace("\r\n", "\n");
    let body = body.strip_suffix('\n').unwrap_or(&body).to_string();
    let get = |k: &str| fields.get(k).and_then(Value::as_str).map(str::to_string);
    let mut missing = Vec::new();
    for k in ["name", "description", "owner"] {
        if get(k).is_none_or(|v| v.trim().is_empty()) {
            missing.push(k);
        }
    }
    if !missing.is_empty() {
        problems.push(format!("front matter needs {}", missing.join(", ")));
        return Ok((problems, None));
    }
    Ok((problems, Some(TemplateSkill { owner: get("owner").unwrap(), name: get("name").unwrap(), description: get("description").unwrap(), body })))
}

#[cfg(test)]
mod tests;
