//! Agent homes and kits from other apps (0.8.0 A1): OpenClaw workspaces, Hermes Agent homes and
//! profiles, Codex homes, Paperclip companies and ai-employees kits. Each arrives as a folder (a
//! zip, a folder pick, a GitHub repo or a folder found on this computer) and is read from a
//! [`Tree`] of the files kwant opened. Only named files are used; key stores (`.env`,
//! `auth.json`, `openclaw.json`, `credentials/`, `config.toml`) are never among them.
//!
//! Sources checked (cited in the PR):
//! - OpenClaw (MIT): docs/concepts/agent-workspace.md, docs/reference/templates/IDENTITY.md
//! - Hermes Agent (MIT): website/docs/user-guide/features/memory.md, profiles.md, skills.md,
//!   tools/skills_sync.py (`.bundled_manifest`)
//! - Codex (Apache-2.0): codex-rs/core/src/agents_md.rs (`AGENTS.override.md` before `AGENTS.md`)
//! - Paperclip (MIT): docs/companies/companies-spec.md (agentcompanies/v1-draft)
//! - ai-employees (MIT): employees/*/employee.json (`ai-employee/1`), SCHEDULE.md, AGENTS.md

use super::{any_s, arr, bundle, front_matter, memories_from_markdown, s, skill_from_markdown, texts};
use crate::db::continuity::MAX_INSTRUCTIONS_CHARS;
use crate::import::archive::Entry;
use crate::import::scrub::drop_secret_fields;
use crate::import::{cut, slug, BotSpec, Bundle, RoutineSpec, SkillSpec, TeamSpec};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const OPENCLAW: &str = "openclaw";
pub const HERMES: &str = "hermes";
pub const CODEX: &str = "codex";
pub const PAPERCLIP: &str = "paperclip";
pub const AI_EMPLOYEES: &str = "ai-employees";

/// The files of one folder, looked up without regard to case. `known` also holds the paths that
/// were seen but not opened (a Codex `config.toml`), which only ever count as markers.
pub struct Tree<'a> {
    files: BTreeMap<String, (&'a str, &'a [u8])>,
    known: BTreeSet<String>,
}

impl<'a> Tree<'a> {
    pub fn new(entries: &'a [Entry], seen: &'a [String]) -> Self {
        let files: BTreeMap<String, (&str, &[u8])> =
            entries.iter().map(|e| (e.path.to_lowercase(), (e.path.as_str(), e.bytes.as_slice()))).collect();
        let known = files.keys().cloned().chain(seen.iter().map(|p| p.to_lowercase())).collect();
        Tree { files, known }
    }

    fn has(&self, path: &str) -> bool {
        self.known.contains(&path.to_lowercase())
    }

    fn text(&self, path: &str) -> Option<&'a str> {
        let (_, bytes) = self.files.get(&path.to_lowercase())?;
        std::str::from_utf8(bytes).ok().map(|t| t.trim_start_matches('\u{feff}'))
    }

    /// The text, or nothing when it is only whitespace.
    fn nonempty(&self, path: &str) -> Option<&'a str> {
        self.text(path).filter(|t| !t.trim().is_empty())
    }

    /// Opened files under `dir` (lower-case path, original path, bytes).
    fn under<'b>(&'b self, dir: &str) -> impl Iterator<Item = (&'b str, &'a str, &'a [u8])> + 'b {
        let prefix = if dir.is_empty() { String::new() } else { format!("{}/", dir.to_lowercase()) };
        self.files.iter().filter(move |(k, _)| k.starts_with(&prefix)).map(|(k, (p, b))| (k.as_str(), *p, *b))
    }

    fn paths(&self) -> impl Iterator<Item = &String> {
        self.known.iter()
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `dir` when `path` ends with `/suffix` (or is `suffix`).
fn root_of<'p>(path: &'p str, suffix: &str) -> Option<&'p str> {
    if path == suffix {
        return Some("");
    }
    path.strip_suffix(suffix).and_then(|p| p.strip_suffix('/'))
}

/// Roots in order, without ones that sit inside another root.
fn outermost(mut roots: Vec<String>) -> Vec<String> {
    roots.sort();
    roots.dedup();
    let all = roots.clone();
    roots.retain(|r| !all.iter().any(|o| o != r && (o.is_empty() || r.starts_with(&format!("{o}/")))));
    roots
}

/// Original-case version of a lower-case dir, for names shown to the person.
fn shown_dir(t: &Tree, lower_dir: &str) -> String {
    t.files
        .values()
        .find_map(|(p, _)| p.to_lowercase().starts_with(&format!("{lower_dir}/")).then(|| p[..lower_dir.len()].to_string()))
        .unwrap_or_else(|| lower_dir.to_string())
}

/// The body after optional `---` front matter.
fn without_front_matter(text: &str) -> &str {
    match front_matter(text) {
        Ok((_, body)) => body,
        Err(_) => text,
    }
}

/// `SKILL.md` files under `dir/skills`: directly (`skills/<name>/SKILL.md`) or, with `nested`, at any
/// depth (Hermes: `skills/<category>/<name>/SKILL.md`). Returns them and how many couldn't be read.
fn skills_in(t: &Tree, dir: &str, nested: bool, leave_out: &BTreeSet<String>) -> (Vec<SkillSpec>, usize, usize) {
    let skills_dir = join(dir, "skills");
    let mut out = Vec::new();
    let (mut unreadable, mut left_out) = (0, 0);
    for (lower, path, bytes) in t.under(&skills_dir) {
        if base(lower) != "skill.md" {
            continue;
        }
        let depth = lower[skills_dir.len() + 1..].matches('/').count();
        if depth < 1 || (!nested && depth != 1) {
            continue;
        }
        let folder = base(parent(lower)).to_string();
        if leave_out.contains(&folder) {
            left_out += 1;
            continue;
        }
        match std::str::from_utf8(bytes).ok().and_then(|text| front_matter(text.trim_start_matches('\u{feff}')).ok()) {
            Some((mut fm, body)) => {
                drop_secret_fields(&mut fm);
                out.push(skill_from_markdown(&fm, body, Some(path)));
            }
            None => unreadable += 1,
        }
    }
    (out, unreadable, left_out)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn note_unreadable(b: &mut Bundle, n: usize) {
    if n > 0 {
        b.omitted.push(format!("{} without valid front matter", plural(n, "SKILL.md", "SKILL.md files")));
    }
}

/// What the files say this folder is, when it is one of the formats here.
pub fn detect(t: &Tree) -> Option<Bundle> {
    let roots = ai_employee_roots(t);
    if !roots.is_empty() {
        return Some(ai_employees(t, &roots));
    }
    let roots = paperclip_roots(t);
    if !roots.is_empty() {
        return Some(paperclip(t, &roots));
    }
    let hermes_roots = hermes_roots(t);
    if !hermes_roots.is_empty() {
        return Some(hermes(t, &hermes_roots));
    }
    let roots = openclaw_roots(t);
    if !roots.is_empty() {
        return Some(openclaw(t, &roots));
    }
    let roots = codex_roots(t);
    if !roots.is_empty() {
        return Some(codex(t, &roots));
    }
    None
}

/// A folder whose kind is already known (found on this computer), read from its root.
pub fn parse_kind(kind: &str, t: &Tree) -> Option<Bundle> {
    let root = vec![String::new()];
    match kind {
        OPENCLAW => Some(openclaw(t, &root)),
        HERMES => Some(hermes(t, &root)),
        CODEX => Some(codex(t, &root)),
        _ => None,
    }
}

// ── OpenClaw ──────────────────────────────────────────────────────────────

fn openclaw_roots(t: &Tree) -> Vec<String> {
    let mut roots = Vec::new();
    for p in t.paths() {
        let Some(dir) = root_of(p, "soul.md").or_else(|| root_of(p, "identity.md")) else { continue };
        let workspace_file = ["agents.md", "user.md", "memory.md", "tools.md", "heartbeat.md"].iter().any(|f| t.has(&join(dir, f)))
            || t.paths().any(|q| q.starts_with(&join(dir, "memory/")));
        if workspace_file {
            roots.push(dir.to_string());
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

/// `- **Name:** Clawd` lines (labels any case; `_(…)_` placeholders and empty values left out).
fn identity_fields(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in without_front_matter(text).lines() {
        let l = line.trim().trim_start_matches(['-', '*']).trim().replace("**", "");
        let Some((label, value)) = l.split_once(':') else { continue };
        let label = label.trim().to_lowercase();
        if !matches!(label.as_str(), "name" | "creature" | "vibe" | "theme" | "emoji" | "avatar") {
            continue;
        }
        let value = value.trim().trim_matches('_').trim();
        if value.is_empty() || (value.starts_with('(') && value.ends_with(')')) {
            continue;
        }
        out.entry(label).or_insert_with(|| value.to_string());
    }
    out
}

fn openclaw(t: &Tree, roots: &[String]) -> Bundle {
    let mut b = bundle(OPENCLAW, Some("OpenClaw".into()));
    let (mut logs, mut boot, mut unreadable) = (0, false, 0);
    for dir in roots {
        let id = t.text(&join(dir, "IDENTITY.md")).map(identity_fields).unwrap_or_default();
        // `workspace-<agentId>` is one agent of a multi-agent setup; `workspace` the default one.
        let shown = shown_dir(t, dir);
        let folder = base(&shown);
        let fallback = match folder.to_lowercase() {
            f if f.is_empty() || f == "workspace" || f.starts_with('.') => "OpenClaw".to_string(),
            f if f.starts_with("workspace-") => folder["workspace-".len()..].to_string(),
            _ => folder.to_string(),
        };
        let name = id.get("name").cloned().unwrap_or(fallback);
        let title = ["theme", "creature", "vibe"].iter().find_map(|k| id.get(*k).cloned()).unwrap_or_else(|| "OpenClaw agent".into());
        let mut persona = t.text(&join(dir, "AGENTS.md")).map(|a| without_front_matter(a).trim().to_string()).unwrap_or_default();
        let extra: Vec<String> =
            [("Creature", "creature"), ("Vibe", "vibe")].iter().filter_map(|(label, k)| id.get(*k).map(|v| format!("- {label}: {v}"))).collect();
        if !extra.is_empty() {
            persona = format!("{persona}\n\n## Identity\n{}", extra.join("\n")).trim().to_string();
        }
        let mut memories = t.text(&join(dir, "MEMORY.md")).map(|m| memories_from_markdown(without_front_matter(m))).unwrap_or_default();
        if let Some(user) = t.text(&join(dir, "USER.md")) {
            memories.extend(memories_from_markdown(without_front_matter(user)).into_iter().map(|m| format!("About the person: {m}")));
        }
        let (skills, bad, _) = skills_in(t, dir, false, &BTreeSet::new());
        unreadable += bad;
        logs += t.paths().filter(|p| p.starts_with(&join(dir, "memory/")) && p.ends_with(".md")).count();
        boot |= t.has(&join(dir, "BOOT.md")) || t.has(&join(dir, "BOOTSTRAP.md"));
        b.bots.push(BotSpec {
            ref_: slug(&name),
            avatar_hint: id.get("emoji").cloned(),
            soul: t.text(&join(dir, "SOUL.md")).map(|s| without_front_matter(s).trim().to_string()).unwrap_or_default(),
            name,
            title,
            persona_prompt: persona,
            memories,
            skills,
            ..BotSpec::default()
        });
    }
    if logs > 0 {
        b.omitted.push(format!("{} (memory/*.md) weren't imported", plural(logs, "daily memory log", "daily memory logs")));
    }
    if boot {
        b.omitted.push("BOOT.md and BOOTSTRAP.md (first-run scripts) weren't imported".into());
    }
    note_unreadable(&mut b, unreadable);
    b
}

// ── Hermes Agent ──────────────────────────────────────────────────────────

fn hermes_roots(t: &Tree) -> Vec<String> {
    let mut roots: Vec<String> = t
        .paths()
        .filter_map(|p| root_of(p, "memories/memory.md").or_else(|| root_of(p, "memories/user.md")).or_else(|| root_of(p, "skills/.bundled_manifest")))
        .map(str::to_string)
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// Hermes memory entries: separated by lines that hold only `§`.
fn hermes_entries(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if line.trim() == "§" {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out.push(cur);
    out.into_iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect()
}

/// Skill names in `skills/.bundled_manifest` (`name:hash` lines, or plain names in v1).
fn bundled_skills(text: &str) -> BTreeSet<String> {
    text.lines().filter_map(|l| l.split(':').next()).map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect()
}

fn hermes(t: &Tree, roots: &[String]) -> Bundle {
    let mut b = bundle(HERMES, Some("Hermes Agent".into()));
    let (mut seeded, mut unreadable) = (0, 0);
    for dir in roots {
        let shown = shown_dir(t, dir);
        // `profiles/<name>/` is a Hermes profile: its own bot, named after it.
        let profile = shown.rsplit_once('/').filter(|(p, _)| base(p).eq_ignore_ascii_case("profiles")).map(|(_, n)| n.to_string());
        let name = profile.clone().unwrap_or_else(|| "Hermes".into());
        let bundled = t.text(&join(dir, "skills/.bundled_manifest")).map(bundled_skills).unwrap_or_default();
        let (skills, bad, left_out) = skills_in(t, dir, true, &bundled);
        seeded += left_out;
        unreadable += bad;
        let mut memories = t.text(&join(dir, "memories/MEMORY.md")).map(hermes_entries).unwrap_or_default();
        if let Some(user) = t.text(&join(dir, "memories/USER.md")) {
            memories.extend(hermes_entries(user).into_iter().map(|m| format!("About the person: {m}")));
        }
        b.bots.push(BotSpec {
            ref_: slug(&name),
            name,
            title: match profile {
                Some(_) => "Hermes Agent profile".into(),
                None => "Hermes Agent".into(),
            },
            soul: t.text(&join(dir, "SOUL.md")).map(|s| s.trim().to_string()).unwrap_or_default(),
            memories,
            skills,
            ..BotSpec::default()
        });
    }
    if seeded > 0 {
        b.omitted.push(format!("{} that Hermes installs itself (skills/.bundled_manifest) weren't imported", plural(seeded, "skill", "skills")));
    }
    b.omitted.push("Hermes keys and settings (.env, auth.json, config.yaml) are never read".into());
    note_unreadable(&mut b, unreadable);
    b
}

// ── Codex ─────────────────────────────────────────────────────────────────

fn codex_roots(t: &Tree) -> Vec<String> {
    let mut roots = Vec::new();
    for p in t.paths() {
        let Some(dir) = root_of(p, "agents.md").or_else(|| root_of(p, "agents.override.md")) else { continue };
        if t.has(&join(dir, "soul.md")) {
            continue;
        }
        if t.has(&join(dir, "config.toml")) || t.has(&join(dir, "agents.override.md")) || base(dir) == ".codex" {
            roots.push(dir.to_string());
        }
    }
    outermost(roots)
}

/// Codex reads `AGENTS.override.md` when it has text, else `AGENTS.md`.
pub fn codex_instructions(override_md: Option<&str>, agents_md: Option<&str>) -> String {
    override_md.filter(|t| !t.trim().is_empty()).or(agents_md).unwrap_or("").trim().to_string()
}

fn codex(t: &Tree, roots: &[String]) -> Bundle {
    let mut b = bundle(CODEX, Some("Codex".into()));
    let mut unreadable = 0;
    for dir in roots {
        let (skills, bad, _) = skills_in(t, dir, false, &BTreeSet::new());
        unreadable += bad;
        b.bots.push(codex_bot(codex_instructions(t.nonempty(&join(dir, "AGENTS.override.md")), t.text(&join(dir, "AGENTS.md"))), skills));
    }
    b.omitted.push("Codex sign-in and settings (auth.json, config.toml) are never read".into());
    note_unreadable(&mut b, unreadable);
    b
}

pub fn codex_bot(instructions: String, skills: Vec<SkillSpec>) -> BotSpec {
    BotSpec {
        ref_: "codex".into(),
        name: "Codex".into(),
        title: "Coding agent (from Codex)".into(),
        persona_prompt: instructions,
        skills,
        ..BotSpec::default()
    }
}

// ── Paperclip ─────────────────────────────────────────────────────────────

fn has_front_matter(t: &Tree, path: &str) -> bool {
    t.text(path).is_some_and(|x| front_matter(x).is_ok())
}

/// `<root>/agents/<slug>/AGENTS.md` → `<root>`.
fn paperclip_agent_root(lower: &str) -> Option<&str> {
    let agent_dir = root_of(lower, "agents.md")?;
    let agents = parent(agent_dir);
    (base(agents) == "agents").then(|| parent(agents))
}

fn paperclip_roots(t: &Tree) -> Vec<String> {
    let mut roots: Vec<String> = t.paths().filter_map(|p| root_of(p, "company.md")).filter(|d| has_front_matter(t, &join(d, "company.md"))).map(str::to_string).collect();
    if roots.is_empty() {
        roots = t.paths().filter_map(|p| root_of(p, "team.md")).filter(|d| has_front_matter(t, &join(d, "team.md"))).map(str::to_string).collect();
    }
    if roots.is_empty() {
        roots = t
            .paths()
            .filter(|p| has_front_matter(t, p))
            .filter_map(|p| paperclip_agent_root(p))
            .map(str::to_string)
            .collect();
    }
    outermost(roots)
}

/// The agent slug a Paperclip path names: `../cto/AGENTS.md` and `agents/cto/AGENTS.md` → `cto`;
/// a bare `cto` stays `cto`.
fn agent_slug_of(reference: &str) -> String {
    let r = reference.trim().replace('\\', "/").to_lowercase();
    match root_of(&r, "agents.md") {
        Some(dir) => base(dir).to_string(),
        None => base(r.trim_end_matches('/')).to_string(),
    }
}

/// One Paperclip agent: its slug (folder, or front matter `slug`), front matter and body.
struct PaperclipAgent {
    slug: String,
    fm: Value,
    body: String,
}

fn paperclip(t: &Tree, roots: &[String]) -> Bundle {
    let mut b = bundle(PAPERCLIP, Some("Paperclip".into()));
    let (mut projects, mut one_off, mut unreadable) = (0, 0, 0);
    for root in roots {
        let prefix = if root.is_empty() { String::new() } else { format!("{root}/") };
        let company = t.text(&join(root, "COMPANY.md")).and_then(|x| front_matter(x).ok());
        let team_md = t.text(&join(root, "TEAM.md")).and_then(|x| front_matter(x).ok());
        projects += t.paths().filter(|p| p.starts_with(&prefix) && base(p) == "project.md").count();

        // Skills by short name: `skills/<shortname>/SKILL.md`.
        let mut skills: BTreeMap<String, SkillSpec> = BTreeMap::new();
        let mut agents: Vec<PaperclipAgent> = Vec::new();
        let mut tasks: Vec<(String, Value, String)> = Vec::new();
        for (lower, path, bytes) in t.under(root) {
            let is_skill = base(lower) == "skill.md" && base(parent(parent(lower))) == "skills";
            let is_agent = paperclip_agent_root(lower).is_some();
            let is_task = base(lower) == "task.md";
            if !(is_skill || is_agent || is_task) {
                continue;
            }
            let Some((mut fm, body)) = std::str::from_utf8(bytes).ok().and_then(|x| front_matter(x.trim_start_matches('\u{feff}')).ok()) else {
                unreadable += usize::from(is_skill);
                continue;
            };
            drop_secret_fields(&mut fm);
            let folder = base(parent(lower)).to_string();
            if is_skill {
                skills.insert(folder, skill_from_markdown(&fm, body, Some(path)));
            } else if is_agent {
                let own = s(&fm, "slug").trim().to_lowercase();
                agents.push(PaperclipAgent { slug: if own.is_empty() { folder } else { own }, fm, body: body.trim().to_string() });
            } else {
                let own = s(&fm, "slug").trim().to_lowercase();
                tasks.push((if own.is_empty() { folder } else { own }, fm, body.trim().to_string()));
            }
        }
        let known: BTreeSet<String> = agents.iter().map(|a| a.slug.clone()).collect();

        // Recurring tasks become routines; their schedule lives in .paperclip.yaml `routines`.
        let triggers = t
            .text(&join(root, ".paperclip.yaml"))
            .and_then(|y| serde_norway::from_str::<Value>(y).ok())
            .and_then(|v| v.get("routines").cloned())
            .unwrap_or(Value::Null);
        let mut routines: BTreeMap<String, Vec<RoutineSpec>> = BTreeMap::new();
        for (task_slug, fm, body) in tasks {
            let recurring = fm.get("recurring").and_then(Value::as_bool).unwrap_or(false);
            let assignee = agent_slug_of(&s(&fm, "assignee"));
            if !recurring || !known.contains(&assignee) {
                one_off += 1;
                continue;
            }
            let schedule_text = arr(triggers.get(&task_slug).unwrap_or(&Value::Null), "triggers")
                .iter()
                .find(|tr| s(tr, "kind") == "schedule" && !s(tr, "cronExpression").trim().is_empty())
                .map(|tr| match s(tr, "timezone").trim() {
                    "" => s(tr, "cronExpression").trim().to_string(),
                    zone => format!("{} ({zone})", s(tr, "cronExpression").trim()),
                });
            let name = any_s(&fm, &["name", "title"]);
            routines.entry(assignee).or_default().push(RoutineSpec {
                name: if name.trim().is_empty() { task_slug } else { name },
                instructions: cut(&body, MAX_INSTRUCTIONS_CHARS),
                schedule_text,
                ..RoutineSpec::default()
            });
        }

        // The lead: TEAM.md's manager, else the agent that reports to nobody.
        let mut lead = team_md.as_ref().map(|(fm, _)| agent_slug_of(&s(fm, "manager"))).filter(|m| known.contains(m));
        let mut used = BTreeSet::new();
        let mut members = Vec::new();
        for a in &agents {
            if lead.is_none() && matches!(a.fm.get("reportsTo"), None | Some(Value::Null)) {
                lead = Some(a.slug.clone());
            }
            let mut own = Vec::new();
            for wanted in texts(arr(&a.fm, "skills")) {
                let w = wanted.trim().replace('\\', "/").to_lowercase();
                let short = base(root_of(&w, "skill.md").unwrap_or(w.trim_end_matches('/'))).to_string();
                if let Some(sk) = skills.get(&short) {
                    own.push(sk.clone());
                    used.insert(short);
                }
            }
            members.push(slug(&a.slug));
            b.bots.push(BotSpec {
                ref_: slug(&a.slug),
                name: any_s(&a.fm, &["name", "slug"]),
                title: any_s(&a.fm, &["title", "role"]),
                persona_prompt: a.body.clone(),
                skills: own,
                routines: routines.remove(&a.slug).unwrap_or_default(),
                ..BotSpec::default()
            });
        }
        b.skills.extend(skills.into_iter().filter(|(k, _)| !used.contains(k)).map(|(_, v)| v));
        if members.len() > 1 {
            let header = team_md.as_ref().or(company.as_ref()).map(|(fm, _)| fm);
            let name = header.map(|fm| s(fm, "name")).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                let d = base(&shown_dir(t, root)).to_string();
                if d.is_empty() { "Paperclip company".into() } else { d }
            });
            let mut instructions: Vec<String> = Vec::new();
            for (fm, _) in [&company, &team_md].into_iter().flatten() {
                let d = s(fm, "description").trim().to_string();
                if !d.is_empty() && !instructions.contains(&d) {
                    instructions.push(d);
                }
            }
            if let Some((fm, _)) = &company {
                let goals = texts(arr(fm, "goals"));
                if !goals.is_empty() {
                    instructions.push(format!("Company goals:\n{}", goals.iter().map(|g| format!("- {}", g.trim())).collect::<Vec<_>>().join("\n")));
                }
            }
            let lead = lead.map(|l| slug(&l));
            b.teams.push(TeamSpec {
                name,
                members: members.into_iter().filter(|r| Some(r) != lead.as_ref()).collect(),
                lead,
                instructions: Some(instructions.join("\n\n")).filter(|i| !i.is_empty()),
            });
        }
    }
    if projects > 0 {
        b.omitted.push(format!("{} weren't imported", plural(projects, "project", "projects")));
    }
    if one_off > 0 {
        b.omitted.push(format!("{} that aren't recurring (or have no agent here) weren't imported", plural(one_off, "task", "tasks")));
    }
    b.omitted.push("Paperclip adapters, environment inputs and secrets (.paperclip.yaml) are never imported".into());
    note_unreadable(&mut b, unreadable);
    b
}

// ── ai-employees ──────────────────────────────────────────────────────────

fn ai_employee_roots(t: &Tree) -> Vec<String> {
    let mut roots: Vec<String> = t
        .paths()
        .filter_map(|p| root_of(p, "employee.json"))
        .filter(|d| {
            t.text(&join(d, "employee.json"))
                .and_then(|x| serde_json::from_str::<Value>(x).ok())
                .is_some_and(|v| s(&v, "schema").starts_with("ai-employee/"))
        })
        .map(str::to_string)
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// `mon-fri` + `07:15` → `15 7 * * 1-5`; days the cron can't say (`first-weekday`, `off`) stay as
/// written, so the routine arrives at the usual placeholder with a warning.
pub fn ai_employee_schedule(days: &str, fire: &str) -> String {
    let days = days.trim().trim_matches('`').to_lowercase();
    let fire = fire.trim().trim_matches('`');
    let dow = match days.as_str() {
        "mon-fri" => Some("1-5"),
        "mon" => Some("1"),
        "tue" => Some("2"),
        "wed" => Some("3"),
        "thu" => Some("4"),
        "fri" => Some("5"),
        "sat" => Some("6"),
        "sun" => Some("0"),
        _ => None,
    };
    let hm = fire.split_once(':').and_then(|(h, m)| Some((h.trim().parse::<u8>().ok()?, m.trim().parse::<u8>().ok()?))).filter(|(h, m)| *h < 24 && *m < 60);
    match (dow, hm) {
        (Some(d), Some((h, m))) => format!("{m} {h} * * {d}"),
        _ => format!("{days} {fire}"),
    }
}

/// `SCHEDULE.md` table rows: `| \`routine\` | \`days\` | fire | …`.
fn schedule_rows(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l.trim().trim_matches('|').split('|').map(|c| c.trim()).collect();
            if cells.len() < 3 || !cells[0].starts_with('`') {
                return None;
            }
            let id = cells[0].trim_matches('`').to_string();
            (cells[2].contains(':') && !id.is_empty()).then(|| (id, cells[1].trim_matches('`').to_string(), cells[2].to_string()))
        })
        .collect()
}

fn ai_employees(t: &Tree, roots: &[String]) -> Bundle {
    let mut b = bundle(AI_EMPLOYEES, Some("AI Employees".into()));
    let mut any_routines = false;
    for dir in roots {
        let Some(mut v) = t.text(&join(dir, "employee.json")).and_then(|x| serde_json::from_str::<Value>(x).ok()) else { continue };
        drop_secret_fields(&mut v);
        let name = any_s(&v, &["name", "slug"]);
        let mut rows: Vec<(String, String, String)> =
            arr(&v, "routines").iter().map(|r| (any_s(r, &["id", "name"]), s(r, "days"), s(r, "fire"))).filter(|(id, _, _)| !id.is_empty()).collect();
        if rows.is_empty() {
            rows = t.text(&join(dir, "SCHEDULE.md")).map(schedule_rows).unwrap_or_default();
        }
        let routines: Vec<RoutineSpec> = rows
            .into_iter()
            .map(|(id, days, fire)| {
                let description = t
                    .text(&join(dir, &format!("routines/{id}/SKILL.md")))
                    .and_then(|x| front_matter(x).ok())
                    .map(|(fm, _)| s(&fm, "description"))
                    .unwrap_or_default();
                let instructions = format!(
                    "{}\n\nFollow routines/{id}/SKILL.md in the {name} kit folder (github.com/markfulton/ai-employees). As the kit's AGENTS.md says, read CONTRACT.md, ROLE.md and your row in SCHEDULE.md first.",
                    description.trim()
                );
                RoutineSpec {
                    name: id,
                    instructions: cut(instructions.trim(), MAX_INSTRUCTIONS_CHARS),
                    schedule_text: Some(ai_employee_schedule(&days, &fire)),
                    ..RoutineSpec::default()
                }
            })
            .collect();
        any_routines |= !routines.is_empty();
        let tool_hints = arr(&v, "connections")
            .iter()
            .map(|c| {
                let providers = texts(arr(c, "providers"));
                if providers.is_empty() { s(c, "capability") } else { format!("{} ({})", s(c, "capability"), providers.join(", ")) }
            })
            .filter(|h| !h.trim().is_empty())
            .collect();
        b.bots.push(BotSpec {
            ref_: slug(&any_s(&v, &["slug", "name"])),
            name,
            title: s(&v, "role"),
            persona_prompt: t.text(&join(dir, "AGENTS.md")).map(|a| a.trim().to_string()).unwrap_or_default(),
            routines,
            tool_hints,
            ..BotSpec::default()
        });
    }
    if any_routines {
        b.omitted.push("AI Employee routines run from the kit's own folder (CONTRACT.md, SCHEDULE.md, scripts); only their names, schedules and a pointer were imported".into());
        b.omitted.push("Routine times are the kit's local machine time and were set in UTC: check each before turning it on".into());
    }
    b
}

#[cfg(test)]
mod tests;
