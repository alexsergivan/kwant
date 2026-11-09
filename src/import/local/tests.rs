use super::*;
use std::fs;
use std::os::unix::fs::symlink;

/// Looks like a key; must never appear in anything kwant imports.
const KEY: &str = "sk-test-NEVER-IMPORT-0123456789abcdef";

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn skill(name: &str) -> String {
    format!("---\nname: {name}\ndescription: Does {name}.\n---\n\nSteps for {name}.\n")
}

/// A home folder with every kind of source, plus key stores next to them.
fn home_with_everything() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path();
    put(h, ".openclaw/workspace/SOUL.md", "Calm and direct.");
    put(h, ".openclaw/workspace/AGENTS.md", "Answer briefly.");
    put(h, ".openclaw/workspace/IDENTITY.md", "- **Name:** Clawdia\n- **Emoji:** 🦞\n");
    put(h, ".openclaw/workspace/MEMORY.md", "- Likes tea\n");
    put(h, ".openclaw/workspace/memory/2026-11-01.md", "daily log");
    put(h, ".openclaw/workspace/skills/weather/SKILL.md", &skill("weather"));
    put(h, ".openclaw/openclaw.json", &format!("{{\"apiKey\": \"{KEY}\"}}"));
    put(h, ".openclaw/credentials/oauth.json", KEY);
    put(h, ".hermes/SOUL.md", "Hermes soul.");
    put(h, ".hermes/memories/MEMORY.md", "Uses metric units\n§\nWorks at night\n");
    put(h, ".hermes/memories/USER.md", "Name is Sam\n");
    put(h, ".hermes/skills/.bundled_manifest", "arxiv:abc123\n");
    put(h, ".hermes/skills/research/arxiv/SKILL.md", &skill("arxiv"));
    put(h, ".hermes/skills/mine/SKILL.md", &skill("mine"));
    put(h, ".hermes/.env", &format!("OPENAI_API_KEY={KEY}\n"));
    put(h, ".hermes/auth.json", KEY);
    put(h, ".hermes/config.yaml", &format!("api_key: {KEY}\n"));
    put(h, ".hermes/profiles/work/SOUL.md", "Work soul.");
    put(h, ".hermes/profiles/work/.env", KEY);
    put(h, ".claude/agents/reviewer.md", "---\nname: reviewer\ndescription: Reviews code.\n---\n\nReview the diff.\n");
    put(h, ".claude/skills/pdf/SKILL.md", &skill("pdf"));
    put(h, ".claude/.credentials.json", KEY);
    put(h, ".codex/AGENTS.md", "Prefer small diffs.");
    put(h, ".codex/auth.json", KEY);
    put(h, ".codex/config.toml", &format!("api_key = \"{KEY}\"\n"));
    tmp
}

fn by_id<'a>(found: &'a [Found], id: &str) -> &'a Found {
    found.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("{id} not found in {found:?}"))
}

#[test]
fn scan_lists_names_and_counts() {
    let tmp = home_with_everything();
    let found = scan(tmp.path(), None);
    let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["openclaw", "hermes", "hermes:work", "claude-agents", "claude-skills", "codex"]);

    let oc = by_id(&found, "openclaw");
    assert_eq!((oc.kind.as_str(), oc.label.as_str(), oc.path.as_str()), ("openclaw", "OpenClaw", "~/.openclaw/workspace"));
    assert_eq!((oc.bots, oc.skills, oc.memories, oc.too_large), (1, 1, 1, false));
    assert_eq!(oc.names, ["weather"]);

    // The skill Hermes bundled itself isn't counted
    let hm = by_id(&found, "hermes");
    assert_eq!((hm.bots, hm.skills, hm.memories), (1, 1, 2));
    assert_eq!(hm.names, ["mine"]);
    assert_eq!(by_id(&found, "hermes:work").label, "Hermes Agent (work profile)");

    let ca = by_id(&found, "claude-agents");
    assert_eq!((ca.kind.as_str(), ca.bots, ca.names.clone()), ("claude-code", 1, vec!["reviewer".to_string()]));
    let cs = by_id(&found, "claude-skills");
    assert_eq!((cs.kind.as_str(), cs.bots, cs.skills), ("skill", 0, 1));
    let cx = by_id(&found, "codex");
    assert_eq!((cx.bots, cx.skills, cx.path.as_str()), (1, 0, "~/.codex"));

    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains(KEY));
}

#[test]
fn key_stores_are_never_wanted() {
    for (kind, rel) in [
        (formats::OPENCLAW, "openclaw.json"),
        (formats::OPENCLAW, "credentials/oauth.json"),
        (formats::OPENCLAW, ".env"),
        (formats::HERMES, ".env"),
        (formats::HERMES, "auth.json"),
        (formats::HERMES, "config.yaml"),
        (formats::HERMES, "state.db"),
        (formats::HERMES, "sessions/abc.json"),
        (formats::CODEX, "auth.json"),
        (formats::CODEX, "config.toml"),
        (formats::CODEX, "sessions/2026/rollout.jsonl"),
        (CLAUDE_AGENTS, ".credentials.json"),
        (CLAUDE_SKILLS, "pdf/scripts/run.py"),
    ] {
        assert_eq!(wanted(kind, rel), None, "{kind} {rel}");
    }
    assert!(!descend(formats::OPENCLAW, "credentials"));
    assert!(!descend(formats::CODEX, "sessions"));
    assert!(!descend(formats::HERMES, "profiles"));
}

#[test]
fn read_merges_sources_and_never_reads_keys() {
    let tmp = home_with_everything();
    // A key file kwant can't open: if import tried, it would be named as skipped
    fs::set_permissions(tmp.path().join(".hermes/auth.json"), std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let ids: Vec<String> = ["openclaw", "hermes", "claude-agents", "claude-skills", "codex"].map(String::from).to_vec();
    let b = read(tmp.path(), None, &ids).unwrap();
    assert_eq!(b.source.kind, "local");
    assert_eq!(b.source.label.as_deref(), Some("Found on this computer: OpenClaw, Hermes Agent, Claude Code agents, Claude Code skills, Codex"));
    let names: Vec<&str> = b.bots.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Clawdia", "Hermes", "reviewer", "Codex"]);
    let hermes = &b.bots[1];
    assert!(hermes.memories.iter().any(|m| m.contains("metric")), "{:?}", hermes.memories);
    assert_eq!(hermes.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["mine"]);
    assert!(b.skills.iter().any(|s| s.name == "pdf"));
    let json = serde_json::to_string(&b).unwrap();
    assert!(!json.contains(KEY), "a key was imported");
    assert!(!json.contains("auth.json") || !json.contains("Skipped"), "a key store was opened: {:?}", b.omitted);
    assert!(!b.omitted.iter().any(|o| o.starts_with("Skipped")), "{:?}", b.omitted);
}

#[test]
fn one_source_keeps_its_kind_and_profiles_are_named() {
    let tmp = home_with_everything();
    let b = read(tmp.path(), None, &["hermes:work".into()]).unwrap();
    assert_eq!(b.source.kind, "hermes");
    assert_eq!(b.source.label.as_deref(), Some("Found on this computer: Hermes Agent (work profile)"));
    assert_eq!((b.bots[0].name.as_str(), b.bots[0].title.as_str()), ("work", "Hermes Agent profile"));
}

#[test]
fn unknown_ids_are_not_found() {
    let tmp = home_with_everything();
    assert_eq!(read(tmp.path(), None, &["../../etc".into()]), Err(ReadError::NotFound("../../etc".into())));
    assert_eq!(read(tmp.path(), None, &["openclaw:nope".into()]), Err(ReadError::NotFound("openclaw:nope".into())));
}

#[test]
fn symlinked_folders_count_as_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    put(elsewhere.path(), "AGENTS.md", "Outside the home folder.");
    put(elsewhere.path(), "SKILL.md", &skill("outside"));
    symlink(elsewhere.path(), tmp.path().join(".codex")).unwrap();
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    symlink(elsewhere.path(), tmp.path().join(".claude/agents")).unwrap();
    // A skill folder that is a link is skipped, the real one next to it read
    put(tmp.path(), ".claude/skills/real/SKILL.md", &skill("real"));
    fs::create_dir_all(elsewhere.path().join("linked")).unwrap();
    put(elsewhere.path(), "linked/SKILL.md", &skill("linked"));
    symlink(elsewhere.path().join("linked"), tmp.path().join(".claude/skills/linked")).unwrap();

    let found = scan(tmp.path(), None);
    assert_eq!(found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["claude-skills"]);
    assert_eq!(found[0].names, ["real"]);
    assert_eq!(read(tmp.path(), None, &["codex".into()]), Err(ReadError::NotFound("codex".into())));

    let b = read(tmp.path(), None, &["claude-skills".into()]).unwrap();
    assert_eq!(b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["real"]);
    assert!(b.omitted.iter().any(|o| o.contains("Skipped") && o.contains("linked")), "{:?}", b.omitted);
}

#[test]
fn a_symlinked_file_is_skipped_and_named() {
    let tmp = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    put(elsewhere.path(), "secret.txt", KEY);
    put(tmp.path(), ".openclaw/workspace/AGENTS.md", "Answer briefly.");
    put(tmp.path(), ".openclaw/workspace/IDENTITY.md", "- **Name:** Clawdia\n");
    symlink(elsewhere.path().join("secret.txt"), tmp.path().join(".openclaw/workspace/SOUL.md")).unwrap();
    let b = read(tmp.path(), None, &["openclaw".into()]).unwrap();
    let json = serde_json::to_string(&b).unwrap();
    assert!(!json.contains(KEY));
    assert!(b.omitted.iter().any(|o| o.contains("Skipped") && o.contains("SOUL.md")), "{:?}", b.omitted);
}

#[test]
fn codex_home_is_honoured_but_not_through_a_link() {
    let tmp = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    put(codex.path(), "AGENTS.md", "From CODEX_HOME.");
    put(tmp.path(), ".codex/AGENTS.md", "From ~/.codex.");
    let real = codex.path().canonicalize().unwrap();
    let b = read(tmp.path(), Some(&real), &["codex".into()]).unwrap();
    assert!(b.bots[0].persona_prompt.contains("From CODEX_HOME."), "{:?}", b.bots[0].persona_prompt);

    let link = tmp.path().join("codex-link");
    symlink(&real, &link).unwrap();
    assert!(scan(tmp.path(), Some(&link)).iter().all(|f| f.id != "codex"));
}

#[test]
fn more_than_500_files_is_too_large() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..=MAX_ZIP_ENTRIES {
        put(tmp.path(), &format!(".claude/agents/a{i:03}.md"), "Be helpful.");
    }
    let found = scan(tmp.path(), None);
    assert!(found[0].too_large);
    assert_eq!(found[0].names.len(), MAX_NAMES);
    assert!(matches!(read(tmp.path(), None, &["claude-agents".into()]), Err(ReadError::Import(ImportError::TooLarge(_)))));
}

#[test]
fn an_empty_home_finds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(scan(tmp.path(), None).is_empty());
    assert!(scan(&tmp.path().join("missing"), None).is_empty());
}
