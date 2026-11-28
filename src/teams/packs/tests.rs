use super::*;
use crate::teams::templates::{self, BUILTIN_PACKS};
use serde::Serialize;
use serde_json::json;

/// The 0.9 `src/teams/templates.json`, kept as the golden copy the built-in packs must match.
const TEMPLATES_09: &str = include_str!("../../../tests/fixtures/team-templates-0.9.json");

fn builtin(id: &str) -> BTreeMap<String, Vec<u8>> {
    let (_, files) = BUILTIN_PACKS.iter().find(|(i, _)| *i == id).expect("built-in pack");
    files.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect()
}

fn edit_json(files: &mut BTreeMap<String, Vec<u8>>, f: impl FnOnce(&mut Value)) {
    let mut v: Value = serde_json::from_slice(&files["pack.json"]).unwrap();
    f(&mut v);
    files.insert("pack.json".into(), serde_json::to_vec_pretty(&v).unwrap());
}

fn problems(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    check_files(files).expect_err("the pack is refused")
}

fn refused_with(files: &BTreeMap<String, Vec<u8>>, needle: &str) {
    let p = problems(files);
    assert!(p.iter().any(|l| l.contains(needle)), "no problem says {needle:?}: {p:#?}");
    assert!(p.iter().all(|l| !l.contains('\n')), "one line per problem: {p:#?}");
}

/// A key-shaped value, built at run time so this file holds none.
fn fake_key() -> String {
    format!("sk-{}", "a1b2".repeat(8))
}

/// A template in the 0.9 `templates.json` shape, hidden fields included, in its key order.
#[derive(Serialize)]
struct T09<'a> {
    id: &'a str,
    label: &'a str,
    icon: &'a str,
    handle: &'a str,
    card_line: &'a str,
    mission: &'a str,
    instructions: &'a str,
    needs: &'a [Need],
    never: &'a [String],
    routine: Option<R09<'a>>,
    first_prompts: &'a [String],
    bots: Vec<B09<'a>>,
    skills: &'a [TemplateSkill],
}

#[derive(Serialize)]
struct R09<'a> {
    name: &'a str,
    when: &'a str,
    cron: &'a str,
    bot: &'a str,
    enabled: bool,
    prompt: &'a str,
}

#[derive(Serialize)]
struct B09<'a> {
    preset: &'a str,
    lead: bool,
    name: &'a str,
    title: &'a str,
    avatar_sprite: &'a str,
    line: &'a str,
    persona: &'a str,
    soul: &'a str,
    good_for: &'a [String],
    not_for: &'a [String],
}

fn as_09(t: &Template) -> T09<'_> {
    assert!(t.routines.len() <= 1, "{}: 0.9 had one routine at most", t.id);
    T09 {
        id: &t.id,
        label: &t.label,
        icon: &t.icon,
        handle: &t.handle,
        card_line: &t.card_line,
        mission: &t.mission,
        instructions: &t.instructions,
        needs: &t.needs,
        never: &t.never,
        routine: t.routines.first().map(|r| R09 { name: &r.name, when: &r.when, cron: &r.cron, bot: &r.bot, enabled: r.enabled, prompt: &r.prompt }),
        first_prompts: &t.first_prompts,
        bots: t
            .bots
            .iter()
            .map(|b| B09 {
                preset: &b.preset,
                lead: b.lead,
                name: &b.name,
                title: &b.title,
                avatar_sprite: &b.avatar_sprite,
                line: &b.line,
                persona: &b.persona,
                soul: &b.soul,
                good_for: &b.good_for,
                not_for: &b.not_for,
            })
            .collect(),
        skills: &t.skills,
    }
}

/// `JSON.stringify(templates, null, 1) + "\n"`, as `scripts/gen-team-data.mjs` wrote 0.9's file.
fn as_09_text(ts: &[Template]) -> String {
    let list: Vec<T09> = ts.iter().map(as_09).collect();
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
    list.serialize(&mut ser).unwrap();
    String::from_utf8(out).unwrap() + "\n"
}

/// 34l: the five 0.9 templates are now packs, and the packs (with no answers) give main's
/// `templates.json` back byte for byte.
#[test]
fn builtin_packs_with_no_answers_are_the_09_templates_byte_for_byte() {
    let ids: Vec<&str> = templates::all().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["helpers", "inbox", "sales", "coding", "home", "local-services"]);
    let old: Vec<Template> = templates::all()[..5].iter().map(|t| t.filled(&HashMap::new())).collect();
    let text = as_09_text(&old);
    if text != TEMPLATES_09 {
        let at = text.bytes().zip(TEMPLATES_09.bytes()).position(|(a, b)| a != b).unwrap_or(text.len().min(TEMPLATES_09.len()));
        let from = at.saturating_sub(80);
        panic!(
            "the packs differ from tests/fixtures/team-templates-0.9.json at byte {at} (lengths {} and {}):\npacks:   {:?}\nfixture: {:?}",
            text.len(),
            TEMPLATES_09.len(),
            text.get(from..(at + 80).min(text.len())).unwrap_or(""),
            TEMPLATES_09.get(from..(at + 80).min(TEMPLATES_09.len())).unwrap_or("")
        );
    }
    // And the 0.9 shape is the same JSON as the fixture, read back (not only the same bytes)
    let golden: Value = serde_json::from_str(TEMPLATES_09).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), golden);
}

#[test]
fn catalogue_is_the_09_catalogue_for_the_first_five() {
    let golden: Vec<Value> = serde_json::from_str(TEMPLATES_09).unwrap();
    assert_eq!(golden.len(), 5);
    for (t, old) in templates::all().iter().zip(golden) {
        // What 0.9 served: the template without its hidden fields, each bot with its slot
        let mut want = old.clone();
        let o = want.as_object_mut().unwrap();
        o.remove("instructions");
        for b in o["bots"].as_array_mut().unwrap() {
            let b = b.as_object_mut().unwrap();
            b.remove("persona");
            b.remove("soul");
            let name = b["name"].clone();
            b.insert("slot".into(), name);
        }
        if let Some(r) = o["routine"].as_object_mut() {
            r.remove("cron");
            r.remove("prompt");
        }
        assert_eq!(serde_json::to_value(t.catalogue()).unwrap(), want, "{}", t.id);
    }
}

/// The Local services pack (34l): asks fill their lines; a create without answers leaves those
/// lines out and asks the person in the chat instead.
#[test]
fn local_services_answers_fill_their_lines_and_no_answer_drops_them() {
    let t = templates::get("local-services").unwrap();
    assert_eq!(t.asks.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["business_name", "what_you_sell", "area"]);
    assert_eq!((t.version.as_str(), t.field.as_str()), ("1.0.0", "Local services"));
    assert!(t.routines.len() >= 2 && t.routines.iter().all(|r| !r.enabled), "{:?}", t.routines);
    let answers = HashMap::from([
        ("business_name".to_string(), "Bright Spark Electrical".to_string()),
        ("what_you_sell".to_string(), "Rewiring, fuse boxes and EV chargers".to_string()),
        ("area".to_string(), "North Leeds".to_string()),
    ]);
    let filled = t.filled(&answers);
    for want in ["Bright Spark Electrical", "Rewiring, fuse boxes and EV chargers", "North Leeds"] {
        assert!(filled.instructions.contains(want), "{want}: {}", filled.instructions);
    }
    let lead = filled.lead();
    assert!(lead.persona.contains("Bright Spark Electrical"), "{}", lead.persona);
    let everything = |t: &Template| {
        let mut all = vec![t.instructions.clone()];
        all.extend(t.bots.iter().flat_map(|b| [b.persona.clone(), b.soul.clone()]));
        all.extend(t.skills.iter().map(|s| s.body.clone()));
        all.extend(t.routines.iter().map(|r| r.prompt.clone()));
        all.join("\n")
    };
    assert!(!everything(&filled).contains("{{"));
    let plain = t.filled(&HashMap::new());
    let text = everything(&plain);
    assert!(!text.contains("{{") && !text.contains("Bright Spark"), "{text}");
    assert!(plain.instructions.contains("we ask you once"), "{}", plain.instructions);
    // The never rules are the four the brief names, in plain words
    let never = t.never.join("\n").to_lowercase();
    for word in ["price", "refund", "angry", "without your ok"] {
        assert!(never.contains(word), "{word}: {never}");
    }
}

#[test]
fn fill_keeps_text_without_placeholders_and_drops_unanswered_lines() {
    let a = HashMap::from([("x".to_string(), "one".to_string()), ("empty".to_string(), String::new())]);
    assert_eq!(fill("no braces\n", &a), "no braces\n");
    assert_eq!(fill("a {{x}} b {{x}}\nkeep\n", &a), "a one b one\nkeep\n");
    assert_eq!(fill("first\n{{y}} gone\nlast", &a), "first\nlast");
    assert_eq!(fill("{{empty}} gone\nlast", &a), "last");
    assert_eq!(fill("last line {{x}}", &a), "last line one");
    assert_eq!(placeholders("{{a_1}} and {{b}}").unwrap(), ["a_1", "b"]);
    assert!(placeholders("{{ spaced }}").is_err());
    assert!(placeholders("open {{x").is_err());
    assert_eq!(clean_answer("  bags\u{200B}\n for shops \u{7} ").as_deref(), Some("bags\n for shops"));
    assert!(clean_answer(&"x".repeat(MAX_ANSWER_CHARS + 1)).is_none());
}

#[test]
fn every_builtin_pack_passes_the_check() {
    for (id, _) in BUILTIN_PACKS {
        let t = check_files(&builtin(id)).unwrap_or_else(|p| panic!("{id}: {p:#?}"));
        assert_eq!(t.id, *id);
        assert_eq!(t.version, "1.0.0");
        assert!(!t.field.is_empty() && !t.readme.is_empty());
    }
}

#[test]
fn refuses_a_secret_in_pack_json_skills_or_readme() {
    let mut f = builtin("inbox");
    edit_json(&mut f, |v| v["instructions"] = json!(format!("Use api_key {} for the mail.", fake_key())));
    refused_with(&f, "pack.json: instructions has what looks like a key");

    let mut f = builtin("inbox");
    let skill = String::from_utf8(f["skills/draft-a-reply.md"].clone()).unwrap() + &format!("\nToken: {}\n", fake_key());
    f.insert("skills/draft-a-reply.md".into(), skill.into_bytes());
    refused_with(&f, "skills/draft-a-reply.md has what looks like a key");

    let mut f = builtin("inbox");
    f.insert("README.md".into(), format!("# Inbox\n\nkey {}\n", fake_key()).into_bytes());
    refused_with(&f, "README.md has what looks like a key");
}

#[test]
fn refuses_tools_mcp_servers_approvals_and_dev_mode() {
    for (key, value, says) in [
        ("tools", json!(["Bash"]), "bots[0].tools adds tools"),
        ("mcp_servers", json!({"files": {"command": "npx"}}), "bots[0].mcp_servers adds MCP servers"),
        ("approvals", json!("never"), "bots[0].approvals changes approvals"),
        ("dev_mode", json!(true), "bots[0].dev_mode turns on Dev mode"),
    ] {
        let mut f = builtin("coding");
        edit_json(&mut f, |v| v["bots"][0][key] = value.clone());
        refused_with(&f, says);
    }
    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["mcpServers"] = json!({}));
    refused_with(&f, "mcpServers adds MCP servers");
    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["devMode"] = json!(true));
    refused_with(&f, "devMode turns on Dev mode");

    let mut f = builtin("coding");
    let skill = String::from_utf8(f["skills/plan-a-change.md"].clone()).unwrap().replacen("---\n", "---\nallowed-tools: \"Bash\"\n", 1);
    f.insert("skills/plan-a-change.md".into(), skill.into_bytes());
    refused_with(&f, "skills/plan-a-change.md: allowed-tools adds tools");

    // Any case or spelling: the keys are compared without case, `_`, `-` or spaces
    for (key, says) in [("Allowed_Tools", "adds tools"), ("MCP Servers", "adds MCP servers"), ("APPROVAL-MODE", "changes approvals"), ("Dev-Mode", "turns on Dev mode")] {
        let mut f = builtin("local-services");
        edit_json(&mut f, |v| v["bots"][2][key] = json!(true));
        refused_with(&f, &format!("bots[2].{key} {says}"));

        let mut f = builtin("local-services");
        let skill = String::from_utf8(f["skills/draft-a-review-reply.md"].clone()).unwrap().replacen("---\n", &format!("---\n{key}: yes\n"), 1);
        f.insert("skills/draft-a-review-reply.md".into(), skill.into_bytes());
        refused_with(&f, &format!("skills/draft-a-review-reply.md: {key} {says}"));
    }
}

#[test]
fn refuses_a_routine_switched_on_or_with_a_bad_cron() {
    let mut f = builtin("inbox");
    edit_json(&mut f, |v| v["routines"][0]["enabled"] = json!(true));
    refused_with(&f, "is switched on; a pack's routines start off");

    let mut f = builtin("inbox");
    edit_json(&mut f, |v| v["routines"][0]["cron"] = json!("every morning"));
    refused_with(&f, "bad cron \"every morning\"");

    let mut f = builtin("inbox");
    edit_json(&mut f, |v| v["routines"][0]["bot"] = json!("Nobody"));
    refused_with(&f, "which isn't a bot in this pack");
}

#[test]
fn reads_the_old_single_routine_but_not_both() {
    let mut f = builtin("inbox");
    edit_json(&mut f, |v| {
        let r = v["routines"][0].clone();
        v.as_object_mut().unwrap().remove("routines");
        v["routine"] = r;
    });
    let t = check_files(&f).unwrap();
    assert_eq!(t.routines.len(), 1);
    assert_eq!(t.routine.as_ref().unwrap().name, t.routines[0].name);

    edit_json(&mut f, |v| v["routines"] = json!([]));
    refused_with(&f, "set routines or routine, not both");
}

#[test]
fn several_routines_are_all_kept() {
    let mut f = builtin("inbox");
    edit_json(&mut f, |v| {
        let mut second = v["routines"][0].clone();
        second["name"] = json!("Evening check");
        second["cron"] = json!("0 18 * * 1-5");
        v["routines"].as_array_mut().unwrap().push(second);
    });
    let t = check_files(&f).unwrap();
    assert_eq!(t.routines.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Inbox digest", "Evening check"]);
    let c = serde_json::to_value(t.catalogue()).unwrap();
    assert_eq!(c["routines"].as_array().unwrap().len(), 2);
    assert_eq!(c["routine"]["name"], "Inbox digest");
    assert!(serde_json::to_value(templates::get("inbox").unwrap().catalogue()).unwrap().get("routines").is_none());
}

#[test]
fn refuses_an_unknown_preset() {
    let mut f = builtin("sales");
    edit_json(&mut f, |v| v["bots"][1]["preset"] = json!("wizard"));
    refused_with(&f, "bot Prospector: unknown preset \"wizard\"");
}

#[test]
fn refuses_a_skill_without_an_owner_bot_or_over_the_size_limit() {
    let mut f = builtin("sales");
    let skill = String::from_utf8(f["skills/find-and-qualify-leads.md"].clone()).unwrap().replace("owner: \"Prospector\"", "owner: \"Ghost\"");
    f.insert("skills/find-and-qualify-leads.md".into(), skill.into_bytes());
    refused_with(&f, "skills/find-and-qualify-leads.md: owner \"Ghost\" isn't a bot in this pack");

    let mut f = builtin("sales");
    let skill = String::from_utf8(f["skills/find-and-qualify-leads.md"].clone()).unwrap().replace("owner: \"Prospector\"\n", "");
    f.insert("skills/find-and-qualify-leads.md".into(), skill.into_bytes());
    refused_with(&f, "front matter needs owner");

    let mut f = builtin("sales");
    let skill = String::from_utf8(f["skills/find-and-qualify-leads.md"].clone()).unwrap() + &"x".repeat(MAX_SKILL_BODY_CHARS);
    f.insert("skills/find-and-qualify-leads.md".into(), skill.into_bytes());
    refused_with(&f, &format!("the limit is {MAX_SKILL_BODY_CHARS}"));
}

#[test]
fn refuses_placeholders_that_are_not_declared_or_not_filled() {
    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["bots"][0]["persona"] = json!("You work on {{repo_name}}."));
    refused_with(&f, "{{repo_name}} isn't declared in asks");

    let mut f = builtin("coding");
    let skill = String::from_utf8(f["skills/plan-a-change.md"].clone()).unwrap() + "\nUse {{style}}.";
    f.insert("skills/plan-a-change.md".into(), skill.into_bytes());
    refused_with(&f, "{{style}} isn't declared in asks");

    let mut f = builtin("local-services");
    edit_json(&mut f, |v| v["card_line"] = json!("Front desk for {{business_name}}"));
    refused_with(&f, "card_line: placeholders only work in");

    // area is only used in the instructions
    let mut f = builtin("local-services");
    edit_json(&mut f, |v| v["instructions"] = json!("No placeholder here."));
    refused_with(&f, "ask area isn't used");

    let mut f = builtin("local-services");
    edit_json(&mut f, |v| v["instructions"] = json!("Works in {{ area }}."));
    refused_with(&f, "malformed placeholder");

    let mut f = builtin("local-services");
    edit_json(&mut f, |v| v["routines"][0]["prompt"] = json!("Answer for {{owner_name}}."));
    refused_with(&f, "{{owner_name}} isn't declared in asks");

    // Declared and used in a soul: fine
    let mut f = builtin("coding");
    edit_json(&mut f, |v| {
        v["asks"] = json!([{"id": "repo_name", "label": "Which repo?", "example": "acme/web"}]);
        v["bots"][0]["soul"] = json!("You care about {{repo_name}}.");
    });
    let t = check_files(&f).unwrap();
    let filled = t.filled(&HashMap::from([("repo_name".to_string(), "acme/web".to_string())]));
    assert_eq!(filled.bots[0].soul, "You care about acme/web.");
    assert_eq!(t.filled(&HashMap::new()).bots[0].soul, "");
}

#[test]
fn refuses_bad_shapes() {
    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["schema_version"] = json!(2));
    refused_with(&f, "this kwant reads schema_version 1");

    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["skills"] = json!([]));
    refused_with(&f, "skills go in skills/*.md");

    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["colour"] = json!("red"));
    refused_with(&f, "unknown field `colour`");

    let mut f = builtin("coding");
    f.remove("README.md");
    refused_with(&f, "README.md is missing");

    let mut f = builtin("coding");
    f.insert("run.sh".into(), b"echo hi".to_vec());
    refused_with(&f, "run.sh doesn't belong in a pack");

    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["needs"][0]["opens"] = json!("shell"));
    refused_with(&f, "opens \"shell\" is not one of");

    let mut f = builtin("coding");
    edit_json(&mut f, |v| v["bots"][1]["lead"] = json!(true));
    refused_with(&f, "exactly one bot must be the lead; 2 are");

    // Hidden files and a licence are fine
    let mut f = builtin("coding");
    f.insert(".DS_Store".into(), vec![0, 1, 2]);
    f.insert("LICENSE".into(), b"MIT".to_vec());
    assert!(check_files(&f).is_ok());
}

#[test]
fn check_report_says_ok_or_one_line_per_problem() {
    let dir = std::env::temp_dir().join(format!("kwnt-pack-check-{}", uuid::Uuid::new_v4()));
    for (path, bytes) in builtin("local-services") {
        let p = dir.join(&path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    let (lines, ok) = check_report(&dir);
    assert!(ok, "{lines:?}");
    assert_eq!(lines, ["ok: local-services 1.0.0 (Local services): 3 bots, 3 skills, 3 routines (off), 3 asks"]);

    let mut v: Value = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).unwrap()).unwrap();
    v["routines"][0]["enabled"] = json!(true);
    v["bots"][0]["tools"] = json!(["Bash"]);
    std::fs::write(dir.join("pack.json"), serde_json::to_vec(&v).unwrap()).unwrap();
    let (lines, ok) = check_report(&dir);
    assert!(!ok);
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert!(lines[..2].iter().all(|l| l.starts_with("problem: ")));
    assert_eq!(lines[2], "2 problems; the pack can't be used until they are fixed");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", dir.join("skills/linked.md")).unwrap();
        let (lines, ok) = check_report(&dir);
        assert!(!ok);
        assert!(lines.iter().any(|l| l.contains("skills/linked.md is a link")), "{lines:#?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
