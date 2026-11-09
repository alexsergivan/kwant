//! Each format from a small folder shaped like its project's own docs (sources in `formats.rs`).

use super::*;
use crate::import::parsers::{parse_file, parse_repo_zip};
use crate::import::{normalize, read_schedule, ImportError};
use std::io::Write;

/// A fake key that must never reach a bundle.
const KEY: &str = "sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ";

pub(crate) fn zip_of(files: &[(&str, &str)], comment: Option<&str>) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (path, text) in files {
            zip.start_file(*path, opts).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        if let Some(c) = comment {
            zip.set_comment(c.to_string()).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn import(files: &[(&str, &str)]) -> Bundle {
    parse_file(&zip_of(files, None), Some("folder.zip")).unwrap()
}

fn no_key(b: &Bundle) {
    let json = serde_json::to_string(b).unwrap();
    assert!(!json.contains(KEY), "a key reached the bundle: {json}");
}

// ── OpenClaw ──────────────────────────────────────────────────────────────

const IDENTITY: &str = "---\nsummary: \"Agent identity record\"\n---\n# IDENTITY.md - Who Am I?\n\n- **Name:** Clawd\n- **Creature:** a space lobster\n- **Vibe:** calm, dry humour\n- **Emoji:** 🦞\n- **Avatar:**\n  _(workspace-relative path, http(s) URL, or data URI)_\n";

#[test]
fn openclaw_workspace() {
    let b = import(&[
        (".openclaw/openclaw.json", &format!("{{\"auth\": {{\"anthropic\": \"{KEY}\"}}}}")),
        (".openclaw/credentials/anthropic.json", &format!("{{\"key\": \"{KEY}\"}}")),
        (".openclaw/workspace/SOUL.md", "Be genuinely helpful."),
        (".openclaw/workspace/AGENTS.md", "# AGENTS.md\nRead SOUL.md first. Keep notes."),
        (".openclaw/workspace/IDENTITY.md", IDENTITY),
        (".openclaw/workspace/USER.md", "# USER.md\n- Name: Oleks\n- Timezone: Europe/Ljubljana"),
        (".openclaw/workspace/MEMORY.md", "# Memory\n- Ships kwant on Mondays.\n- Prefers short replies."),
        (".openclaw/workspace/memory/2026-11-08.md", "Daily log"),
        (".openclaw/workspace/BOOTSTRAP.md", "First run"),
        (".openclaw/workspace/skills/weather/SKILL.md", "---\nname: weather\ndescription: Look up the weather\n---\nUse wttr.in."),
        (".openclaw/workspace-research/SOUL.md", "Dig deep."),
        (".openclaw/workspace-research/AGENTS.md", "You research."),
    ]);
    assert_eq!(b.source.kind, OPENCLAW);
    assert_eq!(b.bots.len(), 2);
    let main = b.bots.iter().find(|x| x.name == "Clawd").expect("named from IDENTITY.md");
    assert_eq!(main.title, "a space lobster");
    assert_eq!(main.avatar_hint.as_deref(), Some("🦞"));
    assert_eq!(main.soul, "Be genuinely helpful.");
    assert!(main.persona_prompt.starts_with("# AGENTS.md\nRead SOUL.md first."));
    assert!(main.persona_prompt.contains("- Vibe: calm, dry humour"));
    assert!(main.memories.contains(&"Ships kwant on Mondays.".to_string()));
    assert!(main.memories.contains(&"About the person: Timezone: Europe/Ljubljana".to_string()));
    assert_eq!(main.skills.len(), 1);
    assert_eq!(main.skills[0].name, "weather");
    let research = b.bots.iter().find(|x| x.name == "research").expect("workspace-<agentId>");
    assert_eq!(research.persona_prompt, "You research.");
    assert!(b.omitted.iter().any(|o| o.contains("1 daily memory log")));
    assert!(b.omitted.iter().any(|o| o.contains("BOOTSTRAP.md")));
    no_key(&b);
    let checked = normalize(b, &[]);
    assert!(checked.problems.is_empty(), "{:?}", checked.problems);
}

#[test]
fn identity_lines() {
    let f = identity_fields(IDENTITY);
    assert_eq!(f.get("name").map(String::as_str), Some("Clawd"));
    assert_eq!(f.get("emoji").map(String::as_str), Some("🦞"));
    assert!(!f.contains_key("avatar"), "placeholder on the next line is ignored");
    let template = "- **Name:**\n  _(pick something you like)_\n- **Theme:** _(e.g. noir)_";
    assert!(identity_fields(template).is_empty());
}

// ── Hermes Agent ──────────────────────────────────────────────────────────

#[test]
fn hermes_home_and_profile() {
    let b = import(&[
        (".hermes/.env", &format!("ANTHROPIC_API_KEY={KEY}")),
        (".hermes/auth.json", &format!("{{\"token\": \"{KEY}\"}}")),
        (".hermes/SOUL.md", "You are Hermes."),
        (".hermes/memories/MEMORY.md", "Project kwant is Rust.\n§\nDeploys go through Ada,\nnever directly.\n§\n"),
        (".hermes/memories/USER.md", "Oleks likes short answers."),
        (".hermes/skills/.bundled_manifest", "arxiv:0f3c\nweb-search:99aa\n"),
        (".hermes/skills/research/arxiv/SKILL.md", "---\nname: arxiv\ndescription: Search arXiv\n---\nSeeded."),
        (".hermes/skills/mine/release-notes/SKILL.md", "---\nname: release-notes\ndescription: Write release notes\n---\nKeep it short."),
        (".hermes/skills/.hub/x/SKILL.md", "---\nname: hub\ndescription: x\n---\nx"),
        (".hermes/profiles/coder/SOUL.md", "You write code."),
        (".hermes/profiles/coder/memories/MEMORY.md", "Uses cargo via heavy."),
    ]);
    assert_eq!(b.source.kind, HERMES);
    assert_eq!(b.bots.len(), 2);
    let home = b.bots.iter().find(|x| x.name == "Hermes").unwrap();
    assert_eq!(home.soul, "You are Hermes.");
    assert_eq!(home.memories, vec!["Project kwant is Rust.", "Deploys go through Ada,\nnever directly.", "About the person: Oleks likes short answers."]);
    assert_eq!(home.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["release-notes"]);
    let coder = b.bots.iter().find(|x| x.name == "coder").unwrap();
    assert_eq!(coder.title, "Hermes Agent profile");
    assert_eq!(coder.memories, vec!["Uses cargo via heavy."]);
    assert!(b.omitted.iter().any(|o| o.starts_with("1 skill that Hermes installs itself")));
    no_key(&b);
}

#[test]
fn hermes_memory_entries() {
    assert_eq!(hermes_entries("a\n§\n\n§\nb\nc"), vec!["a", "b\nc"]);
    assert_eq!(bundled_skills("arxiv:1\nplain\n\n"), ["arxiv", "plain"].iter().map(|s| s.to_string()).collect());
}

// ── Codex ─────────────────────────────────────────────────────────────────

#[test]
fn codex_home() {
    let b = import(&[
        (".codex/AGENTS.md", "Always run the tests."),
        (".codex/config.toml", &format!("model = \"gpt-5\"\napi_key = \"{KEY}\"")),
        (".codex/auth.json", &format!("{{\"OPENAI_API_KEY\": \"{KEY}\"}}")),
        (".codex/skills/review/SKILL.md", "---\nname: review\ndescription: Review a diff\n---\nLook for bugs."),
    ]);
    assert_eq!(b.source.kind, CODEX);
    assert_eq!(b.bots.len(), 1);
    assert_eq!(b.bots[0].name, "Codex");
    assert_eq!(b.bots[0].persona_prompt, "Always run the tests.");
    assert_eq!(b.bots[0].skills[0].name, "review");
    no_key(&b);
}

#[test]
fn codex_override_wins_when_it_has_text() {
    assert_eq!(codex_instructions(Some("Override."), Some("Base.")), "Override.");
    assert_eq!(codex_instructions(Some("  \n"), Some("Base.")), "Base.");
    let b = import(&[("codex/AGENTS.override.md", "Override."), ("codex/AGENTS.md", "Base.")]);
    assert_eq!(b.bots[0].persona_prompt, "Override.");
}

#[test]
fn lone_agents_md_is_one_codex_bot() {
    let b = parse_file(b"# Repo rules\nUse cargo through bin/heavy.", Some("AGENTS.md")).unwrap();
    assert_eq!(b.source.kind, CODEX);
    assert_eq!(b.bots[0].persona_prompt, "# Repo rules\nUse cargo through bin/heavy.");
    let b = parse_file(b"---\nname: CTO\ntitle: Chief Technology Officer\nreportsTo: ceo\n---\nOwn the stack.", Some("agents/cto/AGENTS.md")).unwrap();
    assert_eq!(b.source.kind, PAPERCLIP);
    assert_eq!((b.bots[0].name.as_str(), b.bots[0].title.as_str()), ("CTO", "Chief Technology Officer"));
}

#[test]
fn a_repos_agents_md_is_not_a_bot() {
    let b = import(&[
        ("AGENTS.md", "Notes for coding agents working in this repo."),
        ("skills/seo-audit/SKILL.md", "---\nname: seo-audit\ndescription: Audit a site\n---\nSteps."),
        ("skills/copywriting/SKILL.md", "---\nname: copywriting\ndescription: Write copy\n---\nSteps."),
    ]);
    assert!(b.bots.is_empty());
    assert_eq!(b.skills.len(), 2);
}

// ── Paperclip ─────────────────────────────────────────────────────────────

fn paperclip_company() -> Vec<(&'static str, String)> {
    vec![
        ("acme/COMPANY.md", "---\nname: Acme\ndescription: We ship small tools.\nschema: agentcompanies/v1\ngoals:\n  - Reach 1000 users\n  - Keep costs low\n---\nAbout Acme.".into()),
        ("acme/agents/ceo/AGENTS.md", "---\nname: CEO\ntitle: Chief Executive Officer\nreportsTo: null\nskills:\n  - review\n---\nRun the company.".into()),
        ("acme/agents/cto/AGENTS.md", "---\nname: CTO\ntitle: Chief Technology Officer\nreportsTo: ceo\nskills:\n  - ../../skills/missing/SKILL.md\n---\nOwn the stack.".into()),
        ("acme/skills/review/SKILL.md", "---\nname: review\ndescription: Review work\n---\nCheck it.".into()),
        ("acme/skills/shared-notes/SKILL.md", "---\nname: shared-notes\ndescription: Keep notes\n---\nWrite them down.".into()),
        ("acme/tasks/monday-review/TASK.md", "---\nname: Monday Review\nassignee: ceo\nrecurring: true\n---\nReview last week.".into()),
        ("acme/tasks/kickoff/TASK.md", "---\nname: Kickoff\nassignee: cto\n---\nStart.".into()),
        ("acme/projects/q2/PROJECT.md", "---\nname: Q2\n---\nLaunch.".into()),
        (
            "acme/.paperclip.yaml",
            format!("routines:\n  monday-review:\n    triggers:\n      - kind: schedule\n        cronExpression: \"0 9 * * 1\"\n        timezone: America/Chicago\nagents:\n  ceo:\n    adapter:\n      env:\n        ANTHROPIC_API_KEY: {KEY}\n"),
        ),
    ]
}

#[test]
fn paperclip_company_becomes_a_team() {
    let files = paperclip_company();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let b = import(&refs);
    assert_eq!(b.source.kind, PAPERCLIP);
    assert_eq!(b.bots.len(), 2);
    let ceo = b.bots.iter().find(|x| x.ref_ == "ceo").unwrap();
    assert_eq!(ceo.title, "Chief Executive Officer");
    assert_eq!(ceo.persona_prompt, "Run the company.");
    assert_eq!(ceo.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["review"]);
    assert_eq!(ceo.routines.len(), 1);
    assert_eq!(ceo.routines[0].name, "Monday Review");
    assert_eq!(ceo.routines[0].schedule_text.as_deref(), Some("0 9 * * 1 (America/Chicago)"));
    assert!(read_schedule(&ceo.routines[0]).is_some(), "the cron and zone are read");
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["shared-notes"]);
    let team = &b.teams[0];
    assert_eq!(team.name, "Acme");
    assert_eq!(team.lead.as_deref(), Some("ceo"));
    assert_eq!(team.members, vec!["cto"]);
    let instructions = team.instructions.as_deref().unwrap();
    assert!(instructions.starts_with("We ship small tools."));
    assert!(instructions.contains("Company goals:\n- Reach 1000 users\n- Keep costs low"));
    assert!(b.omitted.iter().any(|o| o == "1 project weren't imported" || o.starts_with("1 project")));
    assert!(b.omitted.iter().any(|o| o.starts_with("1 task that aren't recurring")));
    no_key(&b);
    let checked = normalize(b, &[]);
    assert!(checked.problems.is_empty(), "{:?}", checked.problems);
}

#[test]
fn paperclip_team_manager_is_the_lead() {
    let b = import(&[
        ("eng/TEAM.md", "---\nname: Engineering\ndescription: Platform team\nmanager: ../cto/AGENTS.md\n---\n"),
        ("eng/agents/cto/AGENTS.md", "---\nname: CTO\nreportsTo: ceo\n---\nLead."),
        ("eng/agents/dev/AGENTS.md", "---\nname: Dev\nreportsTo: null\n---\nBuild."),
    ]);
    assert_eq!(b.teams[0].name, "Engineering");
    assert_eq!(b.teams[0].lead.as_deref(), Some("cto"));
    assert_eq!(b.teams[0].members, vec!["dev"]);
}

// ── ai-employees ──────────────────────────────────────────────────────────

const EMPLOYEE: &str = r#"{
  "schema": "ai-employee/1",
  "slug": "seo-employee",
  "name": "SEO/AEO Employee",
  "role": "Search, content and answer visibility",
  "license": "MIT",
  "connections": [
    {"capability": "search.performance.read", "providers": ["openseo", "search-console-api"], "required": false},
    {"capability": "cms.write", "providers": []}
  ],
  "routines": [
    {"id": "seo-standup", "days": "mon-fri", "fire": "07:15", "window_start": "07:00", "window_end": "11:30"},
    {"id": "seo-index-sweep", "days": "tue", "fire": "11:00"},
    {"id": "seo-intake-and-map", "days": "first-weekday", "fire": "14:30"}
  ]
}"#;

#[test]
fn ai_employee_kit() {
    let b = import(&[
        ("ai-employees/employees/seo-employee/employee.json", EMPLOYEE),
        ("ai-employees/employees/seo-employee/AGENTS.md", "# SEO/AEO Employee\nPaste this into the Instructions field."),
        ("ai-employees/employees/seo-employee/routines/seo-standup/SKILL.md", "---\nname: seo-standup\ndescription: Morning standup for the SEO desk\n---\nLong steps…"),
        ("ai-employees/employees/seo-employee/SCHEDULE.md", "| routine | days | fire |\n|---|---|---|\n| `ignored` | `mon` | 01:00 |"),
        ("ai-employees/README.md", "Kits"),
    ]);
    assert_eq!(b.source.kind, AI_EMPLOYEES);
    let bot = &b.bots[0];
    assert_eq!(bot.name, "SEO/AEO Employee");
    assert_eq!(bot.ref_, "seo-employee");
    assert_eq!(bot.title, "Search, content and answer visibility");
    assert!(bot.persona_prompt.contains("Paste this into the Instructions field."));
    assert_eq!(bot.tool_hints, vec!["search.performance.read (openseo, search-console-api)", "cms.write"]);
    let names: Vec<&str> = bot.routines.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["seo-standup", "seo-index-sweep", "seo-intake-and-map"]);
    assert_eq!(bot.routines[0].schedule_text.as_deref(), Some("15 7 * * 1-5"));
    assert!(bot.routines[0].instructions.starts_with("Morning standup for the SEO desk"));
    assert!(bot.routines[0].instructions.contains("routines/seo-standup/SKILL.md"));
    assert_eq!(bot.routines[1].schedule_text.as_deref(), Some("0 11 * * 2"));
    assert!(read_schedule(&bot.routines[2]).is_none(), "first-weekday has no cron; it gets the placeholder");
    assert!(b.omitted.iter().any(|o| o.contains("set in UTC")));
    let checked = normalize(b, &[]);
    assert!(checked.problems.is_empty(), "{:?}", checked.problems);
}

#[test]
fn ai_employee_schedule_rows() {
    assert_eq!(ai_employee_schedule("`mon-fri`", "08:00"), "0 8 * * 1-5");
    assert_eq!(ai_employee_schedule("sun", "23:59"), "59 23 * * 0");
    assert_eq!(ai_employee_schedule("off", "09:00"), "off 09:00");
    assert_eq!(ai_employee_schedule("mon", "25:00"), "mon 25:00");
    let rows = schedule_rows("| `seo-standup` | `mon-fri` | 07:15 | 07:00 |\n|---|---|---|\n| routine | days | fire |");
    assert_eq!(rows, vec![("seo-standup".to_string(), "mon-fri".to_string(), "07:15".to_string())]);
}

// ── GitHub repository archives ────────────────────────────────────────────

#[test]
fn repo_zip_strips_the_top_folder_and_reads_the_licence_and_commit() {
    let bytes = zip_of(
        &[
            ("marketingskills-HEAD/LICENSE", "MIT License\n\nCopyright (c) 2025 Corey Haines"),
            ("marketingskills-HEAD/AGENTS.md", "Notes for agents."),
            ("marketingskills-HEAD/README.md", "Readme"),
            ("marketingskills-HEAD/skills/seo-audit/SKILL.md", "---\nname: seo-audit\ndescription: Audit a site\n---\nSteps."),
            ("marketingskills-HEAD/skills/seo-audit/references/notes.md", "Not a skill"),
            ("marketingskills-HEAD/skills/copywriting/SKILL.md", "---\nname: copywriting\ndescription: Write copy\n---\nSteps."),
            ("marketingskills-HEAD/.github/workflows/ci.json", "{}"),
            ("marketingskills-HEAD/tools/bundle.zip", "PK\u{3}\u{4}not really"),
            ("marketingskills-HEAD/package.json", "{\"name\": \"x\"}"),
        ],
        Some("4e1f0c2d9a7b6e5f4e1f0c2d9a7b6e5f4e1f0c2d"),
    );
    let (b, info) = parse_repo_zip(&bytes, None).unwrap();
    assert!(b.bots.is_empty());
    assert_eq!(b.skills.len(), 2);
    assert_eq!(info.commit.as_deref(), Some("4e1f0c2d9a7b6e5f4e1f0c2d9a7b6e5f4e1f0c2d"));
    let (path, text) = info.licence.unwrap();
    assert_eq!(path, "LICENSE");
    assert!(text.starts_with("MIT License"));
    // One folder of the repo.
    let (b, _) = parse_repo_zip(&bytes, Some("skills/copywriting")).unwrap();
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["copywriting"]);
    assert!(matches!(parse_repo_zip(&bytes, Some("nope")), Err(ImportError::Unsupported(_))));
}

#[test]
fn repo_zip_entry_cap() {
    let names: Vec<String> = (0..crate::import::archive::MAX_REPO_ZIP_ENTRIES + 1).map(|i| format!("r-HEAD/src/f{i}.rs")).collect();
    let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "")).collect();
    assert!(matches!(parse_repo_zip(&zip_of(&files, None), None), Err(ImportError::TooLarge(_))));
    // A repo of mostly code under the cap reads fine: code isn't inflated or counted.
    let mut names: Vec<String> = (0..1500).map(|i| format!("r-HEAD/src/f{i}.md")).collect();
    names.push("r-HEAD/skills/a/SKILL.md".into());
    let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), if n.ends_with("SKILL.md") { "---\nname: a\ndescription: b\n---\nc" } else { "x" })).collect();
    let (b, _) = parse_repo_zip(&zip_of(&files, None), None).unwrap();
    assert_eq!(b.skills.len(), 1);
}

#[test]
fn uploads_still_refuse_nested_archives() {
    let bytes = zip_of(&[("a/SKILL.md", "---\nname: a\ndescription: b\n---\nc"), ("a/inner.zip", "PK")], None);
    assert!(matches!(parse_file(&bytes, Some("x.zip")), Err(ImportError::Invalid(_))));
}
