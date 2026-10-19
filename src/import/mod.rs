//! Bringing bots in from other apps, and taking them out again.
//!
//! Every source (a Claude Code agent, a character card, a Grok Bot bundle, an OpenMausBot
//! package, a pasted export-prompt reply, …) is first turned into one [`Bundle`] in memory.
//! Only that bundle is checked ([`normalize`]), previewed and applied; the upload itself is never
//! stored. See `IMPORT-API.md` in the onboarding notes for the HTTP side.

pub mod archive;
pub mod parsers;
pub mod prompts;
pub mod scrub;

use crate::db::continuity::{clean_memory_text, MAX_INSTRUCTIONS_CHARS, MAX_MEMORIES_PER_KWANT, MAX_MEMORY_CHARS};
use crate::db::skills::{clean_skill_name, clean_skill_text, MAX_PENDING_PER_KWANT, MAX_SKILL_DESCRIPTION_CHARS};
use crate::routines::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const BUNDLE_FORMAT: &str = "kwant.bundle/1";
/// `source.kind` of pasted text that was taken as one bot's instructions.
pub const PLAIN_TEXT_KIND: &str = "plain-text";

/// Pasted text (an export-prompt reply).
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
/// One JSON, Markdown or PNG file.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// A zip as uploaded.
pub const MAX_ZIP_BYTES: usize = 20 * 1024 * 1024;
/// Everything read out of one zip, counted as it is inflated.
pub const MAX_UNZIPPED_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_ZIP_ENTRIES: usize = 500;
pub const MAX_BOTS: usize = 50;
pub const MAX_TEAMS: usize = 10;
/// Instructions (`persona_prompt`) and `soul`, here and on every bot create and edit.
pub const MAX_PERSONA_CHARS: usize = 16_000;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_TITLE_CHARS: usize = 100;
pub const MAX_ROUTINE_NAME_CHARS: usize = 100;
pub const MAX_ROUTINES_PER_BOT: usize = 20;
pub const MAX_STARTERS: usize = 10;
pub const MAX_STARTER_CHARS: usize = 1_000;
pub const MAX_HINTS: usize = 50;
pub const MAX_HINT_CHARS: usize = 200;
pub const MAX_SHARED_SKILLS: usize = 50;
pub const MAX_TEAM_NAME_CHARS: usize = 60;

/// The message every create and edit path gives for an over-long persona or soul.
pub fn persona_too_long(field: &str) -> String {
    format!("{field} can be at most 16,000 characters")
}

/// `None` when instructions and soul fit [`MAX_PERSONA_CHARS`], else the error to show.
pub fn check_persona_caps(persona_prompt: &str, soul: &str) -> Option<String> {
    if persona_prompt.chars().count() > MAX_PERSONA_CHARS {
        return Some(persona_too_long("instructions"));
    }
    if soul.chars().count() > MAX_PERSONA_CHARS {
        return Some(persona_too_long("soul"));
    }
    None
}

/// `kwant.bundle/1`: bots, their memories, skills and routines, and teams, from anywhere.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub format: String,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub bots: Vec<BotSpec>,
    #[serde(default)]
    pub teams: Vec<TeamSpec>,
    /// Skills not tied to one bot; they become the space's shared skills.
    #[serde(default)]
    pub skills: Vec<SkillSpec>,
    /// What the source had that kwant doesn't import, in words.
    #[serde(default)]
    pub omitted: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// `kwant`, `claude-code`, `skill`, `character-card`, `grokbot`, `openmaus`, `openmaus-grok`,
    /// `prompt-reply`, `plain-text`, or what an export-prompt reply says (`chatgpt`, `grok`, …).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub exported_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BotSpec {
    /// The bot's id within the bundle; teams name bots by it.
    #[serde(rename = "ref", default)]
    pub ref_: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
    /// The bot's instructions.
    #[serde(default)]
    pub persona_prompt: String,
    #[serde(default)]
    pub soul: String,
    #[serde(default)]
    pub avatar_hint: Option<String>,
    #[serde(default)]
    pub model_hint: Option<String>,
    /// Conversation starters: shown in the preview; kwant has no field for them yet.
    #[serde(default)]
    pub starters: Vec<String>,
    #[serde(default)]
    pub memories: Vec<String>,
    #[serde(default)]
    pub skills: Vec<SkillSpec>,
    #[serde(default)]
    pub routines: Vec<RoutineSpec>,
    /// Tools the bot used: shown, never connected.
    #[serde(default)]
    pub tool_hints: Vec<String>,
    /// Files the bot had: "not imported, re-attach these".
    #[serde(default)]
    pub knowledge_files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillSpec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutineSpec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    /// A kwant [`Schedule`] as JSON; when absent or invalid, `schedule_text` is read.
    #[serde(default)]
    pub schedule: Option<serde_json::Value>,
    /// The schedule as the source wrote it: a cron line, `09:00`, `daily 09:00`, ….
    #[serde(default)]
    pub schedule_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamSpec {
    #[serde(default)]
    pub name: String,
    /// A bot `ref`.
    #[serde(default)]
    pub lead: Option<String>,
    /// Bot `ref`s.
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub instructions: Option<String>,
}

/// Something the preview tells the person, about one bot (`bot` = its ref) or the whole import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<String>,
    pub message: String,
}

impl Notice {
    fn new(bot: Option<&str>, message: impl Into<String>) -> Self {
        Notice { bot: bot.map(str::to_string), message: message.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rename {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub from: String,
    pub to: String,
}

/// A checked bundle and what the person should know before applying it.
#[derive(Debug, Clone, Serialize)]
pub struct Checked {
    pub bundle: Bundle,
    /// Kept, but worth a look (a removed key, a suspicious line, a routine with no schedule, …).
    pub warnings: Vec<Notice>,
    /// Not imported (too long, too many, invalid).
    pub dropped: Vec<Notice>,
    /// Stop the import until fixed (instructions over 16,000 characters, too many bots).
    pub problems: Vec<Notice>,
    /// Bots that get a new name because the space (or the bundle) already has theirs.
    pub renames: Vec<Rename>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    /// 413
    TooLarge(String),
    /// 415: not a format kwant reads.
    Unsupported(String),
    /// 422: a known format with bad content, or an unsafe archive.
    Invalid(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::TooLarge(m) | ImportError::Unsupported(m) | ImportError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ImportError {}

/// Cuts `s` to at most `max` characters.
pub(crate) fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Control characters, and characters that draw nothing or turn text around (zero-width
/// spaces, U+202E right-to-left override and the other bidi controls, soft hyphens).
pub(crate) fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

/// A one-line label (title, team or routine name): whitespace collapsed, hidden characters gone.
fn one_line(raw: &str, max: usize) -> String {
    let visible: String = raw.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !is_hidden(*c)).collect();
    cut(visible.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), max)
}

/// A bot name kwant accepts: no spaces, `@` or hidden characters, at most [`MAX_NAME_CHARS`].
pub fn clean_bot_name(raw: &str) -> String {
    let joined = raw.split_whitespace().collect::<Vec<_>>().join("-");
    let name: String = joined.chars().filter(|c| *c != '@' && !is_hidden(*c)).collect();
    let name = cut(name.trim_matches('-'), MAX_NAME_CHARS);
    if name.is_empty() {
        "Imported-bot".into()
    } else {
        name
    }
}

/// `name`, or `name-2`, `name-3`, … : the first one nobody in `taken` (lowercase) has.
fn free_name(name: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&name.to_lowercase()) {
        return name.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!("{}{suffix}", cut(name, MAX_NAME_CHARS - suffix.len()))
        })
        .find(|c| !taken.contains(&c.to_lowercase()))
        .unwrap_or_else(|| name.to_string())
}

/// Splits a memory into entries of at most [`MAX_MEMORY_CHARS`]: at sentence ends when it can,
/// else at spaces, else anywhere. Whitespace is collapsed as stored memories are.
pub fn split_memory(text: &str) -> Vec<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while rest.chars().count() > MAX_MEMORY_CHARS {
        let limit = rest.char_indices().nth(MAX_MEMORY_CHARS).map(|(i, _)| i).unwrap_or(rest.len());
        let head = &rest[..limit];
        let at = head
            .rmatch_indices([' '])
            .map(|(i, _)| i)
            .find(|&i| i > 0 && head[..i].ends_with(['.', '!', '?']))
            .or_else(|| head.rfind(' ').filter(|&i| i > 0))
            .unwrap_or(limit);
        out.push(rest[..at].trim().to_string());
        rest = rest[at..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

/// Reads a routine's schedule: kwant JSON first, then the source's text. Imported routines
/// always arrive paused, so a schedule only has to be valid, not right.
pub fn read_schedule(spec: &RoutineSpec) -> Option<Schedule> {
    if let Some(v) = &spec.schedule {
        if let Ok(s) = serde_json::from_value::<Schedule>(v.clone()) {
            if s.validate().is_ok() {
                return Some(s.canonical());
            }
        }
    }
    let text = spec.schedule_text.as_deref()?.trim();
    schedule_from_text(text)
}

fn schedule_from_text(text: &str) -> Option<Schedule> {
    let lower = text.to_lowercase();
    let lower = lower.trim();
    let tz = "UTC".to_string();
    // "daily 09:00", "every day at 9:00", "09:00"
    let time = lower
        .trim_start_matches("every day")
        .trim_start_matches("daily")
        .trim_start_matches(" at")
        .trim();
    if let Some(t) = clock(time) {
        return Some(Schedule::Daily { time: t, tz });
    }
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() == 5 {
        let s = Schedule::Cron { expression: fields.join(" "), tz };
        if s.validate().is_ok() {
            return Some(s.canonical());
        }
    }
    None
}

/// `H:MM` or `HH:MM` as `HH:MM`.
fn clock(s: &str) -> Option<String> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// What kwant puts in place of a schedule it couldn't read: the routine arrives paused anyway.
pub fn placeholder_schedule() -> Schedule {
    Schedule::Daily { time: "09:00".into(), tz: "UTC".into() }
}

/// Text that might be someone steering the bot rather than describing it. Flagged, never removed.
pub fn suspicious(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    if lower.contains("http://") || lower.contains("https://") {
        found.push("contains a web address");
    }
    if ["ignore previous", "ignore all previous", "ignore the above", "disregard previous", "disregard all previous"]
        .iter()
        .any(|p| lower.contains(p))
    {
        found.push("tells the bot to ignore earlier instructions");
    }
    if ["send it to", "send them to", "post it to", "upload it to", "forward it to", "email it to", "exfiltrat"]
        .iter()
        .any(|p| lower.contains(p))
    {
        found.push("tells the bot to send data somewhere");
    }
    if text.contains(['\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}'])
        || text.contains("                                                  ")
        || text.contains("\n\n\n\n\n\n\n\n\n\n")
    {
        found.push("has long or invisible whitespace that could hide text");
    }
    found
}

/// Checks and tidies `bundle` for a space whose bots are called `existing` (any case). Pure: the
/// same input always gives the same result, so apply re-runs it on what the preview returned.
pub fn normalize(mut bundle: Bundle, existing: &[String]) -> Checked {
    let mut warnings = Vec::new();
    let mut dropped = Vec::new();
    let mut problems = Vec::new();
    let mut renames = Vec::new();

    bundle.format = BUNDLE_FORMAT.into();
    bundle.source.kind = cut(bundle.source.kind.trim(), 40);
    bundle.source.label = bundle.source.label.as_deref().map(|l| cut(l.trim(), 200));
    bundle.source.exported_at = bundle.source.exported_at.as_deref().map(|l| cut(l.trim(), 40));

    let mut refs = HashSet::new();
    for (i, bot) in bundle.bots.iter_mut().enumerate() {
        let mut r = cut(bot.ref_.trim(), 80);
        let mut n = i + 1;
        while r.is_empty() || refs.contains(&r) {
            r = format!("bot-{n}");
            n += 1;
        }
        refs.insert(r.clone());
        bot.ref_ = r;
    }

    if bundle.source.kind == PLAIN_TEXT_KIND {
        let who = bundle.bots.first().map(|b| b.ref_.clone());
        warnings.push(Notice::new(who.as_deref(), "No name or memory found — we used the text as this bot's instructions. You can rename it after importing."));
    }

    let removed = scrub::scrub_bundle(&mut bundle);
    for (bot, count) in removed {
        let what = if count == 1 { "something that looked like an API key" } else { "things that looked like API keys" };
        warnings.push(Notice::new(bot.as_deref(), format!("We removed {what}. It shows as [removed key].")));
    }

    if bundle.bots.len() > MAX_BOTS {
        problems.push(Notice::new(None, format!("One import can bring at most {MAX_BOTS} bots; this has {}.", bundle.bots.len())));
    }
    if bundle.teams.len() > MAX_TEAMS {
        problems.push(Notice::new(None, format!("One import can bring at most {MAX_TEAMS} teams; this has {}.", bundle.teams.len())));
    }

    let mut taken: HashSet<String> = existing.iter().map(|n| n.to_lowercase()).collect();
    for bot in bundle.bots.iter_mut() {
        let r = bot.ref_.clone();
        let who = Some(r.as_str());

        let wanted = clean_bot_name(&bot.name);
        let name = free_name(&wanted, &taken);
        if name != wanted {
            renames.push(Rename { ref_: r.clone(), from: wanted.clone(), to: name.clone() });
        } else if wanted != bot.name.trim() {
            let shown: String = bot.name.trim().chars().filter(|c| !is_hidden(*c)).collect();
            warnings.push(Notice::new(
                who,
                if shown == wanted {
                    format!("We removed hidden characters from the name {wanted}.")
                } else {
                    format!("Bot names can't have spaces or @, so \"{shown}\" becomes {wanted}.")
                },
            ));
        }
        taken.insert(name.to_lowercase());
        bot.name = name;

        bot.title = one_line(&bot.title, MAX_TITLE_CHARS);
        if bot.title.is_empty() {
            bot.title = "Imported bot".into();
        }
        bot.persona_prompt = bot.persona_prompt.replace("\r\n", "\n").trim().to_string();
        bot.soul = bot.soul.replace("\r\n", "\n").trim().to_string();
        if bot.persona_prompt.is_empty() {
            warnings.push(Notice::new(who, "This bot has no instructions; it will act on its title alone until you add some."));
        }
        for (field, text) in [("instructions", &bot.persona_prompt), ("soul", &bot.soul)] {
            let n = text.chars().count();
            if n > MAX_PERSONA_CHARS {
                problems.push(Notice::new(who, format!("{} ({n} now). Shorten it where it came from (or in the pasted reply), then try the import again.", persona_too_long(&format!("{}'s {field}", bot.name)))));
            }
            for why in suspicious(text) {
                warnings.push(Notice::new(who, format!("The {field} {why}. Read it before importing.")));
            }
        }
        bot.avatar_hint = bot.avatar_hint.as_deref().map(|a| cut(a.trim(), 40)).filter(|a| !a.is_empty());
        bot.model_hint = bot.model_hint.as_deref().map(|a| cut(a.trim(), 100)).filter(|a| !a.is_empty());
        bot.starters = tidy_list(&bot.starters, MAX_STARTERS, MAX_STARTER_CHARS);
        bot.tool_hints = tidy_list(&bot.tool_hints, MAX_HINTS, MAX_HINT_CHARS);
        bot.knowledge_files = tidy_list(&bot.knowledge_files, MAX_HINTS, MAX_HINT_CHARS);
        if let Some(model) = bot.model_hint.as_deref().filter(|m| !m.eq_ignore_ascii_case("auto")) {
            warnings.push(Notice::new(who, format!("It used the model {model}; here it starts on auto. Pick a model after importing.")));
        }
        if !bot.tool_hints.is_empty() {
            warnings.push(Notice::new(who, format!("It used tools ({}). Imported bots start with no tools; connect the ones it needs.", bot.tool_hints.join(", "))));
        }
        if !bot.knowledge_files.is_empty() {
            dropped.push(Notice::new(who, format!("Files aren't brought over: {}. Attach them again if the bot needs them.", bot.knowledge_files.join(", "))));
        }
        if !bot.starters.is_empty() {
            dropped.push(Notice::new(who, format!("{} conversation starter(s): kwant has no place for them yet.", bot.starters.len())));
        }

        let mut memories: Vec<String> = bot.memories.iter().flat_map(|m| split_memory(m)).collect();
        if memories.len() > bot.memories.iter().filter(|m| !m.trim().is_empty()).count() {
            warnings.push(Notice::new(who, format!("Long memories were split into entries of at most {MAX_MEMORY_CHARS} characters.")));
        }
        let cap = MAX_MEMORIES_PER_KWANT as usize;
        if memories.len() > cap {
            for m in &memories[cap..] {
                dropped.push(Notice::new(who, format!("Memory (over the {cap}-memory limit): {}", cut(m, 80))));
            }
            memories.truncate(cap);
        }
        memories.retain(|m| clean_memory_text(m).is_ok());
        bot.memories = memories;

        bot.skills = tidy_skills(std::mem::take(&mut bot.skills), MAX_PENDING_PER_KWANT as usize, who, &mut warnings, &mut dropped);
        bot.routines = tidy_routines(std::mem::take(&mut bot.routines), who, &mut warnings, &mut dropped);
    }
    bundle.skills = tidy_skills(std::mem::take(&mut bundle.skills), MAX_SHARED_SKILLS, None, &mut warnings, &mut dropped);

    let mut team_names = HashSet::new();
    let teams = std::mem::take(&mut bundle.teams);
    for mut team in teams {
        team.name = one_line(&team.name, MAX_TEAM_NAME_CHARS);
        let mut members: Vec<String> = Vec::new();
        for m in team.lead.iter().chain(team.members.iter()) {
            let m = m.trim().to_string();
            if refs.contains(&m) && !members.contains(&m) {
                members.push(m);
            } else if !refs.contains(&m) {
                warnings.push(Notice::new(None, format!("Team {}: {m} isn't a bot in this import, so it was left out.", team.name)));
            }
        }
        if team.name.is_empty() || members.is_empty() || !team_names.insert(team.name.to_lowercase()) {
            dropped.push(Notice::new(None, format!("Team \"{}\": it needs a unique name and at least one imported bot.", team.name)));
            continue;
        }
        team.lead = members.first().cloned();
        team.members = members;
        team.instructions = team.instructions.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| {
            if s.chars().count() > MAX_INSTRUCTIONS_CHARS {
                warnings.push(Notice::new(None, format!("Team {}'s instructions were cut to {MAX_INSTRUCTIONS_CHARS} characters.", team.name)));
            }
            cut(s, MAX_INSTRUCTIONS_CHARS)
        });
        bundle.teams.push(team);
    }

    bundle.omitted = tidy_list(&bundle.omitted, MAX_HINTS, MAX_HINT_CHARS);
    Checked { bundle, warnings, dropped, problems, renames }
}

fn tidy_list(items: &[String], max_items: usize, max_chars: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        let i = cut(i.trim(), max_chars);
        if !i.is_empty() && !out.contains(&i) {
            out.push(i);
        }
    }
    out.truncate(max_items);
    out
}

fn tidy_skills(
    skills: Vec<SkillSpec>,
    max: usize,
    who: Option<&str>,
    warnings: &mut Vec<Notice>,
    dropped: &mut Vec<Notice>,
) -> Vec<SkillSpec> {
    let mut out: Vec<SkillSpec> = Vec::new();
    for s in skills {
        let slug = slug(&s.name);
        let description = if s.description.trim().is_empty() {
            first_line(&s.body)
        } else {
            s.description.clone()
        };
        let description = cut(description.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), MAX_SKILL_DESCRIPTION_CHARS);
        let mut name = slug.clone();
        let mut n = 2;
        while out.iter().any(|o| o.name == name) {
            name = format!("{}-{n}", cut(&slug, 44));
            n += 1;
        }
        match clean_skill_text(&name, &description, &s.body) {
            Ok(text) if out.len() < max => {
                for why in suspicious(&text.body) {
                    warnings.push(Notice::new(who, format!("Skill {}: it {why}.", text.name)));
                }
                out.push(SkillSpec { name: text.name, description: text.description, body: text.body });
            }
            Ok(text) => dropped.push(Notice::new(who, format!("Skill {} (more than {max} skills).", text.name))),
            Err(e) => dropped.push(Notice::new(who, format!("Skill {}: {e}.", if s.name.trim().is_empty() { "(unnamed)" } else { s.name.trim() }))),
        }
    }
    out
}

fn tidy_routines(
    routines: Vec<RoutineSpec>,
    who: Option<&str>,
    warnings: &mut Vec<Notice>,
    dropped: &mut Vec<Notice>,
) -> Vec<RoutineSpec> {
    let mut out = Vec::new();
    for r in routines {
        let name = one_line(&r.name, MAX_ROUTINE_NAME_CHARS);
        let instructions = r.instructions.trim().to_string();
        let label = if name.is_empty() { "(unnamed)".to_string() } else { name.clone() };
        if name.is_empty() || instructions.is_empty() {
            dropped.push(Notice::new(who, format!("Routine {label}: it needs a name and instructions.")));
            continue;
        }
        if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
            dropped.push(Notice::new(who, format!("Routine {label}: instructions are longer than {MAX_INSTRUCTIONS_CHARS} characters.")));
            continue;
        }
        if out.len() >= MAX_ROUTINES_PER_BOT {
            dropped.push(Notice::new(who, format!("Routine {label} (more than {MAX_ROUTINES_PER_BOT} routines).")));
            continue;
        }
        let schedule = match read_schedule(&r) {
            Some(s) => s,
            None => {
                warnings.push(Notice::new(
                    who,
                    format!("Routine {label}: we couldn't read its schedule ({}), so it arrives paused at daily 09:00 UTC. Set the time before turning it on.", r.schedule_text.as_deref().unwrap_or("none")),
                ));
                placeholder_schedule()
            }
        };
        out.push(RoutineSpec {
            name,
            instructions,
            schedule: serde_json::to_value(&schedule).ok(),
            schedule_text: r.schedule_text.map(|t| cut(t.trim(), 200)),
        });
    }
    out
}

/// A skill name as kwant stores it, from whatever the source called it.
pub(crate) fn slug(raw: &str) -> String {
    let mut s = String::new();
    for c in raw.trim().trim_start_matches('/').to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = cut(s.trim_matches('-'), 48).trim_end_matches('-').to_string();
    match clean_skill_name(&s) {
        Ok(s) => s,
        Err(_) => "imported-skill".into(),
    }
}

/// The first non-empty line, without Markdown heading marks.
pub(crate) fn first_line(text: &str) -> String {
    text.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && *l != "---")
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memories_split_at_sentences_and_fit() {
        let long = "One sentence here. ".repeat(60);
        let parts = split_memory(&long);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_MEMORY_CHARS && p.ends_with('.')));
        let no_spaces = "x".repeat(1200);
        let parts = split_memory(&no_spaces);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_MEMORY_CHARS));
    }

    #[test]
    fn names_are_cleaned_and_renamed_on_clash() {
        assert_eq!(clean_bot_name("  Code Reviewer @x "), "Code-Reviewer-x");
        assert_eq!(clean_bot_name("   "), "Imported-bot");
        // IMPORT-QA.md F5: right-to-left override, zero-width and control characters go.
        assert_eq!(clean_bot_name("Evil\u{202E}Name\u{200B}\u{7}"), "EvilName");
        assert_eq!(one_line(" Team\u{2066} A\u{202E}\tB ", 40), "Team A B");
        let taken: HashSet<String> = ["ada".into(), "ada-2".into()].into_iter().collect();
        assert_eq!(free_name("Ada", &taken), "Ada-3");
    }

    #[test]
    fn schedules_from_text() {
        assert_eq!(schedule_from_text("09:30"), Some(Schedule::Daily { time: "09:30".into(), tz: "UTC".into() }));
        assert_eq!(schedule_from_text("daily 7:05"), Some(Schedule::Daily { time: "07:05".into(), tz: "UTC".into() }));
        assert!(matches!(schedule_from_text("0 9 * * 1-5"), Some(Schedule::Cron { .. })));
        assert_eq!(schedule_from_text("whenever it feels right"), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("/Release Checklist!"), "release-checklist");
        assert_eq!(slug("???"), "imported-skill");
    }
}
