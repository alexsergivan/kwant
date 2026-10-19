//! Turning each supported source into a [`Bundle`]. Parsers only map fields; limits, names and
//! secrets are handled once, in [`super::normalize`].
//!
//! | source | how it's recognised |
//! |---|---|
//! | `kwant.bundle/1` | JSON `"format": "kwant.bundle/1"` |
//! | Claude Code agent | Markdown with `---` front matter (`name`, `description`, `tools`, `model`) |
//! | Agent Skill | a `SKILL.md` (front matter `name`, `description`) |
//! | Character Card V1/V2/V3 | JSON `"spec": "chara_card_v2"/"chara_card_v3"` or V1 fields; PNG `ccv3`/`chara` chunk |
//! | Grok Bot | JSON `"format": "grokbot-bundle/v1"` |
//! | OpenMausBot | JSON `"format": "openmaus.package"` (v1, v2), or `"openmaus.grok/1"` |
//! | export-prompt reply | pasted text holding one of the JSON forms above, usually in a ```json block |
//! | plain instructions | any other pasted text: one bot, the text as its instructions |
//! | zip | any mix of the files above |

use super::archive::{self, PNG_SIGNATURE};
use super::scrub::drop_secret_fields;
use super::{
    cut, first_line, BotSpec, Bundle, ImportError, RoutineSpec, SkillSpec, Source, TeamSpec, BUNDLE_FORMAT, MAX_FILE_BYTES,
    MAX_TEXT_BYTES, MAX_ZIP_BYTES, PLAIN_TEXT_KIND,
};
use serde_json::Value;

const ZIP_MAGIC: [u8; 4] = [b'P', b'K', 3, 4];
const EMPTY_ZIP_MAGIC: [u8; 4] = [b'P', b'K', 5, 6];
const MAX_FRONT_MATTER_BYTES: usize = 64 * 1024;

fn unsupported() -> ImportError {
    ImportError::Unsupported(
        "kwant can't read this. Use a kwant bundle, a Claude Code agent or SKILL.md, a character card (JSON or PNG), \
         a Grok Bot or OpenMausBot export, or paste the reply to the export prompt."
            .into(),
    )
}

fn invalid(msg: impl Into<String>) -> ImportError {
    ImportError::Invalid(msg.into())
}

fn too_large(what: &str, limit: &str) -> ImportError {
    ImportError::TooLarge(format!("{what} can be at most {limit}."))
}

/// An uploaded file. `filename` helps tell a SKILL.md from an agent.
pub fn parse_file(bytes: &[u8], filename: Option<&str>) -> Result<Bundle, ImportError> {
    if bytes.starts_with(&ZIP_MAGIC) || bytes.starts_with(&EMPTY_ZIP_MAGIC) {
        if bytes.len() > MAX_ZIP_BYTES {
            return Err(too_large("A zip", "20 MB"));
        }
        return from_zip(bytes);
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(too_large("A file", "4 MB"));
    }
    if bytes.starts_with(&PNG_SIGNATURE) {
        return from_value(archive::card_json_from_png(bytes)?);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| unsupported())?;
    from_text(text, filename)
}

/// Pasted text: usually the reply to the export prompt. Text that isn't one of the formats above
/// and doesn't try to be (a bot's Instructions field copied as is) becomes one bot's instructions.
pub fn parse_pasted(text: &str) -> Result<Bundle, ImportError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(too_large("Pasted text", "256 KB"));
    }
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Err(invalid("There's nothing to import in this text. Paste your bot's reply or its instructions."));
    }
    match from_text(text, None) {
        Err(ImportError::Invalid(_) | ImportError::Unsupported(_)) if !looks_structured(text) => Ok(from_plain_text(text)),
        other => other,
    }
}

/// Whether pasted text means to be JSON or front matter, so a broken reply still gets its error
/// instead of being taken as instructions.
fn looks_structured(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('{') || t.starts_with("---") || t.contains("```json") || t.contains("\"format\"") || t.contains("kwant.bundle/")
}

/// One bot whose instructions are the whole text. Its name and title are the defaults
/// [`super::normalize`] gives; it warns that nothing else was found.
fn from_plain_text(text: &str) -> Bundle {
    Bundle {
        format: BUNDLE_FORMAT.into(),
        source: Source { kind: PLAIN_TEXT_KIND.into(), ..Source::default() },
        bots: vec![BotSpec { ref_: "main".into(), name: "Imported-bot".into(), persona_prompt: text.to_string(), ..BotSpec::default() }],
        ..Bundle::default()
    }
}

fn from_text(text: &str, filename: Option<&str>) -> Result<Bundle, ImportError> {
    let text = text.trim_start_matches('\u{feff}');
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        let v: Value = serde_json::from_str(trimmed).map_err(|e| invalid(format!("This JSON isn't valid ({e}).")))?;
        return from_value(v);
    }
    if trimmed.starts_with("---") {
        return from_markdown(trimmed, filename);
    }
    let json = json_block(text).ok_or_else(|| {
        invalid("We couldn't find a JSON block in this text. Paste the whole reply, including the ```json part.")
    })?;
    let v: Value = serde_json::from_str(json).map_err(|e| invalid(format!("The JSON block in this reply isn't valid ({e}).")))?;
    let mut b = from_value(v)?;
    if b.source.kind.is_empty() || b.source.kind == "kwant" {
        b.source.kind = "prompt-reply".into();
    }
    Ok(b)
}

/// The JSON in a pasted reply: a ```json fence (or any fence starting with `{`), else the span
/// from the first `{` to the last `}`.
pub fn json_block(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        let body_start = after.find('\n').map(|i| i + 1).unwrap_or(after.len());
        let body = &after[body_start..];
        let close = body.find("```").unwrap_or(body.len());
        let inner = body[..close].trim();
        // An unclosed fence runs to the end of the reply; the object still ends at its last brace.
        if let (true, Some(end)) = (inner.starts_with('{'), inner.rfind('}')) {
            return Some(&inner[..=end]);
        }
        rest = &body[(close + 3).min(body.len())..];
    }
    let (a, b) = (text.find('{')?, text.rfind('}')?);
    (a < b).then(|| &text[a..=b])
}

/// Any of the JSON sources.
pub fn from_value(mut v: Value) -> Result<Bundle, ImportError> {
    drop_secret_fields(&mut v);
    let format = v.get("format").and_then(Value::as_str).unwrap_or("").to_string();
    let spec = v.get("spec").and_then(Value::as_str).unwrap_or("").to_string();
    match (format.as_str(), spec.as_str()) {
        (BUNDLE_FORMAT, _) => serde_json::from_value::<Bundle>(v).map_err(|e| invalid(format!("This kwant bundle isn't valid ({e})."))),
        ("grokbot-bundle/v1", _) => Ok(from_grokbot(&v)),
        ("openmaus.package", _) => from_openmaus_package(&v),
        ("openmaus.grok/1", _) => Ok(from_openmaus_grok(&v)),
        (_, "chara_card_v2" | "chara_card_v3") => Ok(from_card(v.get("data").unwrap_or(&Value::Null), &spec)),
        _ if v.get("first_mes").is_some() && v.get("name").is_some() => Ok(from_card(&v, "chara_card_v1")),
        _ => Err(unsupported()),
    }
}

// ── helpers over loose JSON ───────────────────────────────────────────────

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// The first non-empty string field of `keys`.
fn any_s(v: &Value, keys: &[&str]) -> String {
    keys.iter().map(|k| s(v, k)).find(|t| !t.trim().is_empty()).unwrap_or_default()
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Strings, or objects' `text`/`content`/`name`, from a list.
fn texts(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            Value::String(t) => t.clone(),
            other => any_s(other, &["text", "content", "name", "id", "label"]),
        })
        .filter(|t| !t.trim().is_empty())
        .collect()
}

fn bundle(kind: &str, label: Option<String>) -> Bundle {
    Bundle {
        format: BUNDLE_FORMAT.into(),
        source: Source { kind: kind.into(), label, exported_at: None },
        ..Bundle::default()
    }
}

/// Memories written as a Markdown file: one per bullet or paragraph, headings left out.
fn memories_from_markdown(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut para = String::new();
    for line in text.lines() {
        let t = line.trim();
        let bullet = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "));
        if t.is_empty() || t.starts_with('#') || bullet.is_some() {
            if !para.trim().is_empty() {
                out.push(std::mem::take(&mut para).trim().to_string());
            }
            para.clear();
            if let Some(b) = bullet {
                para.push_str(b);
            }
            continue;
        }
        para.push(' ');
        para.push_str(t);
    }
    if !para.trim().is_empty() {
        out.push(para.trim().to_string());
    }
    out
}

// ── Markdown: Claude Code agents and SKILL.md ─────────────────────────────

/// Front matter as JSON, and the body after it.
fn front_matter(text: &str) -> Result<(Value, &str), ImportError> {
    let rest = text.trim_start().strip_prefix("---").ok_or_else(unsupported)?;
    let end = rest.find("\n---").ok_or_else(|| invalid("The front matter (between the --- lines) isn't closed."))?;
    let yaml = &rest[..end];
    if yaml.len() > MAX_FRONT_MATTER_BYTES {
        return Err(too_large("Front matter", "64 KB"));
    }
    let body = rest[end + 4..].trim_start_matches('-');
    let body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
    let v: Value = serde_norway::from_str(yaml).map_err(|_| invalid("The front matter isn't valid YAML."))?;
    if !v.is_object() {
        return Err(invalid("The front matter isn't a list of fields."));
    }
    Ok((v, body))
}

fn is_skill_file(filename: Option<&str>) -> bool {
    filename.map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f).eq_ignore_ascii_case("SKILL.md")).unwrap_or(false)
}

fn from_markdown(text: &str, filename: Option<&str>) -> Result<Bundle, ImportError> {
    let (mut fm, body) = front_matter(text)?;
    drop_secret_fields(&mut fm);
    let is_agent_fm = ["tools", "model", "color"].iter().any(|k| fm.get(k).is_some());
    if is_skill_file(filename) || (!is_agent_fm && fm.get("license").is_some()) {
        let mut b = bundle("skill", None);
        b.skills.push(skill_from_markdown(&fm, body, filename));
        return Ok(b);
    }
    Ok(claude_code_agent(&fm, body, filename))
}

fn skill_from_markdown(fm: &Value, body: &str, filename: Option<&str>) -> SkillSpec {
    let mut name = s(fm, "name");
    if name.trim().is_empty() {
        // skills/<name>/SKILL.md
        name = filename.and_then(|f| f.replace('\\', "/").rsplit('/').nth(1).map(str::to_string)).unwrap_or_default();
    }
    SkillSpec { name, description: s(fm, "description"), body: body.to_string() }
}

/// A Claude Code subagent (`.claude/agents/<name>.md`).
fn claude_code_agent(fm: &Value, body: &str, filename: Option<&str>) -> Bundle {
    let file_stem = filename
        .map(|f| f.replace('\\', "/"))
        .and_then(|f| f.rsplit('/').next().map(|n| n.trim_end_matches(".md").to_string()))
        .unwrap_or_default();
    let name = Some(s(fm, "name")).filter(|n| !n.trim().is_empty()).unwrap_or(file_stem);
    let description = s(fm, "description");
    let sentence = description.split(['.', '\n']).next().unwrap_or("").trim().to_string();
    let tools = match fm.get("tools") {
        Some(Value::String(t)) => t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Some(Value::Array(items)) => texts(items),
        _ => Vec::new(),
    };
    let mut persona = body.trim().to_string();
    if description.chars().count() > sentence.chars().count() + 1 {
        persona = format!("{persona}\n\n## When to use this bot\n{}", description.trim());
    }
    let mut b = bundle("claude-code", None);
    b.bots.push(BotSpec {
        ref_: super::slug(&name),
        name,
        title: if sentence.is_empty() { "Claude Code agent".into() } else { sentence },
        persona_prompt: persona.trim().to_string(),
        model_hint: Some(s(fm, "model")).filter(|m| !m.is_empty()),
        tool_hints: tools,
        ..BotSpec::default()
    });
    b
}

// ── Character cards ───────────────────────────────────────────────────────

/// Character Card V1 (fields at the top), V2 and V3 (fields under `data`).
fn from_card(d: &Value, spec: &str) -> Bundle {
    let name = s(d, "name");
    let fill = |t: String| {
        t.replace("{{original}}", "")
            .replace("{{char}}", &name)
            .replace("{{Char}}", &name)
            .replace("<BOT>", &name)
            .replace("{{user}}", "the user")
            .replace("{{User}}", "the user")
            .replace("<USER>", "the user")
            .trim()
            .to_string()
    };
    let mut parts = Vec::new();
    for (heading, key) in [
        ("", "system_prompt"),
        ("## Character", "description"),
        ("## Scenario", "scenario"),
        ("## Example dialogue", "mes_example"),
        ("## Always remember", "post_history_instructions"),
    ] {
        let t = fill(s(d, key));
        if !t.is_empty() {
            parts.push(if heading.is_empty() { t } else { format!("{heading}\n{t}") });
        }
    }
    let mut starters = vec![fill(s(d, "first_mes"))];
    starters.extend(texts(arr(d, "alternate_greetings")).into_iter().map(fill));
    starters.retain(|t| !t.is_empty());
    let mut omitted = Vec::new();
    if d.get("character_book").is_some_and(|b| !b.is_null()) {
        omitted.push("Lorebook (character_book): kwant has no lorebooks".to_string());
    }
    if !arr(d, "assets").is_empty() {
        omitted.push("Card assets (images, sounds)".to_string());
    }
    let mut b = bundle("character-card", Some(spec.to_string()));
    b.bots.push(BotSpec {
        ref_: super::slug(&name),
        name: name.clone(),
        title: Some(fill(s(d, "nickname"))).filter(|t| !t.is_empty()).unwrap_or_else(|| "Character".into()),
        persona_prompt: parts.join("\n\n"),
        soul: fill(s(d, "personality")),
        starters,
        ..BotSpec::default()
    });
    b.omitted = omitted;
    b
}

// ── Grok Bot ──────────────────────────────────────────────────────────────

/// `grokbot-bundle/v1` from pi-grokbot-import.
fn from_grokbot(v: &Value) -> Bundle {
    let src = v.get("source").cloned().unwrap_or(Value::Null);
    let p = v.get("persona").cloned().unwrap_or(Value::Null);
    let name = any_s(&p, &["name"]);
    let mut b = bundle("grokbot", Some(any_s(&src, &["url", "id"])).filter(|l| !l.is_empty()));
    b.source.exported_at = Some(s(v, "exportedAt")).filter(|t| !t.is_empty());
    let memories = arr(v, "memories").iter().map(|m| any_s(m, &["text", "content"])).filter(|t| !t.trim().is_empty()).collect();
    let skills = arr(v, "skills")
        .iter()
        .map(|sk| SkillSpec { name: s(sk, "name"), description: s(sk, "description"), body: any_s(sk, &["body", "instructions", "content"]) })
        .collect();
    let routines = arr(v, "routines")
        .iter()
        .map(|r| RoutineSpec {
            name: s(r, "name"),
            instructions: any_s(r, &["prompt", "instructions"]),
            schedule: None,
            schedule_text: Some(any_s(r, &["schedule", "cron", "time"])).filter(|t| !t.is_empty()),
        })
        .collect();
    let tool_hints = arr(v, "plugins")
        .iter()
        .map(|pl| {
            let (id, note) = (s(pl, "id"), s(pl, "note"));
            if note.is_empty() { id } else { format!("{id}: {note}") }
        })
        .filter(|t| !t.is_empty())
        .collect();
    b.bots.push(BotSpec {
        ref_: super::slug(&name),
        name,
        title: any_s(&p, &["title"]),
        persona_prompt: any_s(&p, &["instructions", "systemPrompt", "description"]),
        avatar_hint: Some(any_s(&p, &["avatarShape"])).filter(|t| !t.is_empty()),
        memories,
        skills,
        routines,
        tool_hints,
        ..BotSpec::default()
    });
    b.omitted = texts(arr(v, "omitted"));
    if s(&src, "mode") == "metadata-only" {
        b.omitted.push("This Grok Bot export was metadata-only, so its instructions may be missing".into());
    }
    b
}

// ── OpenMausBot ───────────────────────────────────────────────────────────

/// A package schedule as kwant JSON, else its words.
fn openmaus_schedule(sch: &Value) -> (Option<Value>, Option<String>) {
    let days = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    match s(sch, "type").as_str() {
        "cron" => (
            Some(serde_json::json!({"type": "cron", "expression": s(sch, "expression"), "tz": any_s(sch, &["timeZone", "tz"])})),
            Some(s(sch, "expression")),
        ),
        "daily" => {
            let wd: Vec<String> = arr(sch, "weekdays").iter().filter_map(Value::as_u64).filter_map(|d| days.get(d as usize)).map(|d| d.to_string()).collect();
            let time = s(sch, "time");
            if wd.is_empty() || wd.len() == 7 {
                (Some(serde_json::json!({"type": "daily", "time": time, "tz": "UTC"})), Some(time))
            } else {
                (Some(serde_json::json!({"type": "weekly", "time": time, "weekdays": wd, "tz": "UTC"})), Some(time))
            }
        }
        "interval" => (
            Some(serde_json::json!({"type": "interval", "every_minutes": sch.get("everyMinutes").and_then(Value::as_i64).unwrap_or(60)})),
            None,
        ),
        "once" => {
            let at = sch.get("at").and_then(Value::as_i64).and_then(chrono::DateTime::from_timestamp_millis);
            (at.map(|at| serde_json::json!({"type": "once", "at": at.to_rfc3339()})), None)
        }
        _ => (None, Some(sch.to_string()).filter(|t| t != "null")),
    }
}

/// `openmaus.package` v1 and v2: bots, their playbooks and skills, routines, and the team.
fn from_openmaus_package(v: &Value) -> Result<Bundle, ImportError> {
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(0);
    if !(1..=2).contains(&version) {
        return Err(invalid("This OpenMausBot package version isn't supported."));
    }
    let pkg = v.get("package").ok_or_else(|| invalid("This OpenMausBot package has no contents."))?;
    let mut b = bundle("openmaus", Some(any_s(pkg, &["name", "id"])).filter(|t| !t.is_empty()));
    let skills_by_name: Vec<SkillSpec> = pkg
        .get("skills")
        .map(|sk| arr(sk, "entries"))
        .unwrap_or(&[])
        .iter()
        .map(|e| SkillSpec { name: s(e, "name"), description: s(e, "description"), body: any_s(e, &["instructions", "body"]) })
        .collect();
    let playbooks: Vec<(String, SkillSpec)> = arr(pkg, "playbooks")
        .iter()
        .map(|p| (s(p, "key"), SkillSpec { name: any_s(p, &["key", "name"]), description: any_s(p, &["summary", "name"]), body: s(p, "instructions") }))
        .collect();
    let mut used = std::collections::HashSet::new();
    for a in arr(pkg, "agents") {
        let key = s(a, "key");
        let mut skills = Vec::new();
        for name in texts(arr(a, "skills")) {
            if let Some(sk) = skills_by_name.iter().find(|sk| sk.name == name) {
                skills.push(sk.clone());
                used.insert(name);
            }
        }
        for pb in texts(arr(a, "playbooks")) {
            if let Some((_, sk)) = playbooks.iter().find(|(k, _)| *k == pb) {
                skills.push(sk.clone());
            }
        }
        let mut memories = Vec::new();
        if let Some(Value::Object(files)) = a.get("seed").and_then(|sd| sd.get("memory")) {
            for text in files.values().filter_map(Value::as_str) {
                memories.extend(memories_from_markdown(text));
            }
        }
        let soul = s(a, "soul");
        let description = s(a, "description");
        b.bots.push(BotSpec {
            ref_: key,
            name: s(a, "name"),
            title: s(a, "title"),
            persona_prompt: if soul.trim().is_empty() { description } else { soul },
            avatar_hint: a.get("appearance").map(|ap| s(ap, "color")).filter(|c| !c.is_empty()),
            memories,
            skills,
            tool_hints: texts(arr(a, "connections")),
            ..BotSpec::default()
        });
    }
    b.skills = skills_by_name.into_iter().filter(|sk| !used.contains(&sk.name)).collect();
    for r in arr(pkg, "routines") {
        let (schedule, schedule_text) = openmaus_schedule(r.get("schedule").unwrap_or(&Value::Null));
        let spec = RoutineSpec { name: s(r, "name"), instructions: any_s(r, &["prompt", "instructions"]), schedule, schedule_text };
        let agent = s(r, "agent");
        match b.bots.iter_mut().find(|bot| bot.ref_ == agent) {
            Some(bot) => bot.routines.push(spec),
            None => b.omitted.push(format!("Routine {} (its bot isn't in the package)", spec.name)),
        }
    }
    if let Some(team) = pkg.get("team").filter(|t| t.is_object()) {
        let leader = Some(s(team, "leader")).filter(|l| !l.is_empty()).or_else(|| Some(s(pkg, "chiefOfStaff")).filter(|l| !l.is_empty()));
        b.teams.push(TeamSpec {
            name: s(team, "name"),
            lead: leader,
            members: b.bots.iter().map(|bot| bot.ref_.clone()).collect(),
            instructions: Some(s(team, "brief")).filter(|t| !t.is_empty()),
        });
    }
    for (key, what) in [("rooms", "Rooms"), ("connections", "Connected apps (sign in again in kwant)"), ("presets", "Preset bots"), ("examples", "Examples")] {
        if !arr(pkg, key).is_empty() {
            b.omitted.push(what.into());
        }
    }
    Ok(b)
}

/// `openmaus.grok/1`: an OpenMausBot bot's reply to its Grok export prompt. Its field list
/// isn't published, so common names are read and the rest is ignored.
fn from_openmaus_grok(v: &Value) -> Bundle {
    let bot = v.get("bot").filter(|b| b.is_object()).unwrap_or(v);
    let name = any_s(bot, &["name"]);
    let mut b = bundle("openmaus-grok", None);
    let skills = arr(bot, "skills")
        .iter()
        .map(|sk| match sk {
            Value::String(t) => SkillSpec { name: first_line(t), description: String::new(), body: t.clone() },
            o => SkillSpec { name: s(o, "name"), description: s(o, "description"), body: any_s(o, &["instructions", "body", "content"]) },
        })
        .collect();
    let routines = arr(bot, "routines")
        .iter()
        .map(|r| RoutineSpec {
            name: any_s(r, &["name", "title"]),
            instructions: any_s(r, &["prompt", "instructions", "task"]),
            schedule: None,
            schedule_text: Some(any_s(r, &["schedule", "cron", "time", "when"])).filter(|t| !t.is_empty()),
        })
        .collect();
    let mut tool_hints = texts(arr(bot, "connectedApps"));
    tool_hints.extend(texts(arr(bot, "apps")));
    tool_hints.extend(texts(arr(bot, "connected_apps")));
    let memories = match bot.get("memories").or_else(|| bot.get("memory")) {
        Some(Value::Array(items)) => texts(items),
        Some(Value::String(t)) => memories_from_markdown(t),
        _ => Vec::new(),
    };
    b.bots.push(BotSpec {
        ref_: super::slug(&name),
        name,
        title: any_s(bot, &["title", "role", "description"]),
        persona_prompt: any_s(bot, &["instructions", "soul", "systemPrompt", "description"]),
        memories,
        skills,
        routines,
        tool_hints,
        ..BotSpec::default()
    });
    b
}

// ── zip ───────────────────────────────────────────────────────────────────

fn from_zip(bytes: &[u8]) -> Result<Bundle, ImportError> {
    let unzipped = archive::read_zip(bytes)?;
    let mut merged = bundle("zip", None);
    let mut skills = Vec::new();
    let mut kinds = std::collections::BTreeSet::new();
    let mut unread = Vec::new();
    for e in &unzipped.entries {
        let lower = e.path.to_lowercase();
        if is_skill_file(Some(&e.path)) {
            match std::str::from_utf8(&e.bytes).ok().and_then(|t| front_matter(t).ok()) {
                Some((fm, body)) => {
                    skills.push(skill_from_markdown(&fm, body, Some(&e.path)));
                    kinds.insert("skill".to_string());
                }
                None => unread.push(e.path.clone()),
            }
            continue;
        }
        if e.bytes.starts_with(&ZIP_MAGIC) || e.bytes.starts_with(&EMPTY_ZIP_MAGIC) {
            return Err(invalid("This zip contains another archive. Unpack it and import the inner file instead."));
        }
        if lower.ends_with(".txt") || lower.ends_with("readme.md") {
            unread.push(e.path.clone());
            continue;
        }
        match parse_file(&e.bytes, Some(&e.path)) {
            Ok(b) => {
                kinds.insert(b.source.kind.clone());
                merged.bots.extend(b.bots);
                merged.teams.extend(b.teams);
                merged.skills.extend(b.skills);
                merged.omitted.extend(b.omitted);
            }
            Err(ImportError::Unsupported(_)) => unread.push(e.path.clone()),
            Err(err) => return Err(invalid(format!("{}: {err}", e.path))),
        }
    }
    // Skills next to a single agent are that agent's; otherwise the space shares them.
    match merged.bots.as_mut_slice() {
        [only] => only.skills.extend(skills),
        _ => merged.skills.extend(skills),
    }
    if merged.bots.is_empty() && merged.skills.is_empty() {
        return Err(ImportError::Unsupported(
            "This zip has no bots or skills kwant can read. Chat-history exports aren't imported.".into(),
        ));
    }
    if kinds.len() == 1 {
        merged.source.kind = kinds.into_iter().next().unwrap_or_default();
    }
    let not_read: Vec<String> = unzipped.skipped.into_iter().chain(unread).collect();
    if !not_read.is_empty() {
        let shown: Vec<String> = not_read.iter().take(10).cloned().collect();
        let more = if not_read.len() > 10 { format!(" and {} more", not_read.len() - 10) } else { String::new() };
        merged.omitted.push(cut(&format!("Files in the zip that weren't read: {}{more}", shown.join(", ")), 200));
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_block_from_reply() {
        let reply = "Sure! Here it is:\n```json\n{\"format\": \"kwant.bundle/1\"}\n```\nAnything else?";
        assert_eq!(json_block(reply), Some("{\"format\": \"kwant.bundle/1\"}"));
        assert_eq!(json_block("prefix {\"a\": 1} suffix"), Some("{\"a\": 1}"));
        assert_eq!(json_block("no json here"), None);
        assert_eq!(json_block("```json\n{\"a\": 1}\nthe reply was cut"), Some("{\"a\": 1}"));
    }

    #[test]
    fn pasted_plain_text_is_one_bot_and_broken_replies_keep_their_error() {
        let b = parse_pasted("\u{feff}You review pull requests. Be kind.").unwrap();
        assert_eq!(b.source.kind, crate::import::PLAIN_TEXT_KIND);
        assert_eq!(b.bots.len(), 1);
        assert_eq!(b.bots[0].persona_prompt, "You review pull requests. Be kind.");
        // Braces in instructions aren't taken for a broken reply.
        assert_eq!(parse_pasted("Greet {user} by name.").unwrap().bots[0].persona_prompt, "Greet {user} by name.");
        assert!(matches!(parse_pasted("   \n"), Err(ImportError::Invalid(_))));
        assert!(matches!(parse_pasted("{ \"format\": "), Err(ImportError::Invalid(_))));
        assert!(matches!(parse_pasted("Here:\n```json\n{ nope }\n```"), Err(ImportError::Invalid(_))));
        // A file upload doesn't fall back: an unknown file stays unknown.
        assert!(parse_file(b"just words", Some("notes.txt")).is_err());
    }

    #[test]
    fn memory_markdown() {
        let m = memories_from_markdown("# Memory\n- likes tea\n- lives in\n  Kyiv\n\nA paragraph\nover lines.");
        assert_eq!(m, vec!["likes tea", "lives in Kyiv", "A paragraph over lines."]);
    }
}
