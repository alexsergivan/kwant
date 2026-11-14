//! Modern web guidance (web-guidance brief): when a Dev mode space's team folder has web code,
//! kwant offers the built-in `modern-web` skill once, as a pending skill shared with every bot in
//! the space, through the usual Skills review. The skill has a bot look up Google Chrome's Modern
//! Web Guidance with a pinned `npx` command before it writes browser code.
//!
//! kwant itself never runs `npx`: it only reads the team folders' top two levels, on the poller's
//! full pass and when Dev mode is turned on, never on a page load. The offer is marked on
//! `dev_mode_spaces.web_guidance_offered_at` when it is made, so it is never made again in that
//! space. The one `npx` form the skill runs is let through without a question by
//! `crate::mcp::shell_risk` (see [`VERSION`]).

use crate::db::skills::{clean_skill_text, SkillText};
use crate::db::spaces::SpaceId;
use crate::server::continuity::{notify, Notification};
use crate::server::AppState;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The `modern-web-guidance` package version the skill runs, pinned: a new version is a new
/// review. `crate::mcp::shell_risk` lets through only this one.
pub const VERSION: &str = "1.0.0";
/// The npm package the skill runs.
pub const PACKAGE: &str = "modern-web-guidance";
pub const SKILL_NAME: &str = "modern-web";
/// What the relevance matcher reads (`crate::server::skills::relevance`): words a request for
/// browser UI work uses, and none a backend one does.
pub const SKILL_DESCRIPTION: &str = "Modern web UI work: HTML, CSS and JavaScript in the browser: layout, responsive mobile page, form, button, menu, dialog, modal, popover, dark mode, animation, accessibility, frontend performance.";
/// The skill's body stays under this, so it costs little in a turn's prompt.
pub const MAX_BODY_CHARS: usize = 2_500;

/// How long the look at a team folder may take before kwant gives up on it.
const SCAN_LIMIT: Duration = Duration::from_secs(3);
/// At most this many directory entries are looked at per folder.
const MAX_ENTRIES: usize = 4_000;
/// Files that mean web code: a `package.json`, or one of these extensions.
const WEB_EXTENSIONS: [&str; 6] = ["html", "css", "tsx", "jsx", "vue", "svelte"];
/// Folders never looked into: dependencies and build output.
const SKIPPED_DIRS: [&str; 2] = ["node_modules", "target"];

/// The skill's body; `{v}` is [`VERSION`].
const BODY: &str = r#"Adapted from Google Chrome's Modern Web Guidance (Apache-2.0, github.com/GoogleChrome/modern-web-guidance). Offered by kwant's Dev mode.

Use this before you write or change web UI code that runs in a browser: HTML, CSS, client-side JavaScript or TypeScript, and components (React, Vue, Svelte). Not for backend, SQL, infra, CI, scripts, build config or tests.

1. Search the guides with 2 to 5 words about the feature:
   npx -y modern-web-guidance@{v} search "accessible modal dialog"
   If nothing fits, rephrase once.
2. Retrieve the best match by its id:
   npx -y modern-web-guidance@{v} retrieve <id>
   When it says the guide was saved to a file under /tmp, read that file in full before going on.
3. Follow the guide: use the platform features it recommends, check their Baseline support, and keep the fallbacks it gives for browsers without them. Where it clashes with the project's own conventions or browser targets, follow the project and say why.
4. End your reply with one line per guide you used:
   Modern Web Guidance: <id>, <what it changed>

Rules
- Run only these exact forms, each on its own: `npx -y modern-web-guidance@{v} search|retrieve|list ...`, or `pnpx modern-web-guidance@{v} ...` in a pnpm project. Never @latest or another version, never legacy-scan, no extra flags, no chaining with &&, ; or |. Anything else asks the person first.
- It needs Node and the network the first time. Give the command a 90-second limit if your shell tool takes one.
- If it fails, is missing or takes more than 90 seconds, say once "Couldn't load Modern Web Guidance here" and carry on with the task. Don't retry in a loop, and never install Node."#;

/// The skill's checked text.
pub fn skill_text() -> SkillText {
    clean_skill_text(SKILL_NAME, SKILL_DESCRIPTION, &BODY.replace("{v}", VERSION)).expect("the built-in modern-web skill is valid")
}

/// Whether `dir` has web code in its top two levels: a `package.json` or a file with one of
/// [`WEB_EXTENSIONS`]. Hidden entries, [`SKIPPED_DIRS`] and symlinks are skipped, and at most
/// [`MAX_ENTRIES`] entries are read. Blocking; see [`has_web_code_limited`].
pub fn has_web_code(dir: &Path) -> bool {
    let mut seen = 0usize;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    if scan_level(dir, &mut seen, Some(&mut subdirs)) {
        return true;
    }
    subdirs.into_iter().any(|d| scan_level(&d, &mut seen, None))
}

/// One folder's entries: true on a web file; its subfolders go to `subdirs` when given.
fn scan_level(dir: &Path, seen: &mut usize, mut subdirs: Option<&mut Vec<PathBuf>>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return false;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        // The entry's own type: a symlink is neither a file nor a folder here
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if let Some(out) = subdirs.as_deref_mut() {
                if !SKIPPED_DIRS.contains(&name) {
                    out.push(entry.path());
                }
            }
        } else if kind.is_file() && is_web_file(name) {
            return true;
        }
    }
    false
}

fn is_web_file(name: &str) -> bool {
    if name == "package.json" {
        return true;
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => WEB_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        _ => false,
    }
}

/// [`has_web_code`] off the async threads, giving up (false) after [`SCAN_LIMIT`].
pub async fn has_web_code_limited(dir: PathBuf) -> bool {
    let scan = tokio::task::spawn_blocking(move || has_web_code(&dir));
    matches!(tokio::time::timeout(SCAN_LIMIT, scan).await, Ok(Ok(true)))
}

/// Offers the `modern-web` skill to `space` once, when Dev mode is on there and a team's folder
/// has web code. The person sees a `skill_proposed` notification named for that team's lead
/// (its Review opens that bot's Skills), and the pending skill in every bot's Skills, where they
/// approve or reject it. Never offered twice in a space, and kwant runs nothing here.
pub async fn offer_if_web(state: &AppState, space: &SpaceId) {
    match state.db.dev_mode(space.as_str()).await {
        Ok(Some(row)) if row.on && row.web_guidance_offered_at.is_none() => {}
        _ => return,
    }
    let teams = state.db.list_teams(space).await.unwrap_or_default();
    let space_dir = crate::db::spaces::space_data_dir(&state.config.data_dir, space);
    let mut found = None;
    for team in &teams {
        let link = team.repo_url.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let folder = crate::tools::team_workspace_path(&space_dir, &team.name, team.workspace_path.as_deref(), link);
        if has_web_code_limited(folder).await {
            found = Some(team);
            break;
        }
    }
    let Some(team) = found else { return };
    let skill = match state.db.offer_web_guidance_once(space.as_str(), &skill_text()).await {
        Ok(Some(s)) => s,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(space = %space, "dev: could not offer the modern-web skill: {e}");
            return;
        }
    };
    tracing::info!(space = %space, team = %team.name, skill = %skill.id, "dev: offered the modern-web skill");
    crate::server::skills::changed(state, &skill).await;
    let lead = state.db.get_kwant(&team.lead_kwant_id).await.ok().flatten();
    if let Some(lead) = lead {
        notify(
            state,
            Notification {
                kind: "skill_proposed",
                kwant_id: lead.id.clone(),
                kwant_name: lead.name.clone(),
                channel_id: None,
                title: "A skill for web work".into(),
                body: format!(
                    "{}'s folder has web code. Review the {SKILL_NAME} skill in Skills: it has every bot in this space look up Modern Web Guidance before writing browser code.",
                    team.name
                ),
                thread_id: None,
                card_id: None,
            },
        )
        .await;
    }
}

/// [`offer_if_web`] in its own task, so neither a request nor the poll waits on the folder look.
pub fn offer_soon(state: &AppState, space: &SpaceId) {
    let (state, space) = (state.clone(), space.clone());
    tokio::spawn(async move { offer_if_web(&state, &space).await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::skills::{Skill, MAX_SKILL_DESCRIPTION_CHARS};
    use crate::server::skills::{prompt_block_for, select};

    fn as_skill() -> Skill {
        let t = skill_text();
        let now = chrono::Utc::now();
        Skill {
            id: "s1".into(),
            kwant_id: None,
            name: t.name,
            description: t.description,
            body: t.body,
            status: "enabled".into(),
            source: "agent".into(),
            proposed_by: None,
            replaces_id: None,
            created_at: now,
            updated_at: now,
            reviewed_at: None,
            space_id: "default".into(),
        }
    }

    #[test]
    fn the_skill_text_is_within_its_limits_and_pins_one_version() {
        let t = skill_text();
        assert_eq!(t.name, SKILL_NAME);
        assert!(t.description.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS, "{}", t.description.chars().count());
        assert!(t.body.chars().count() <= MAX_BODY_CHARS, "body is {} chars", t.body.chars().count());
        assert!(t.body.starts_with("Adapted from Google Chrome's Modern Web Guidance (Apache-2.0"));
        assert!(!t.body.contains("{v}"));
        assert!(t.body.contains(&format!("npx -y {PACKAGE}@{VERSION} search")));
        assert!(t.body.contains(&format!("npx -y {PACKAGE}@{VERSION} retrieve")));
        assert!(t.body.contains("Couldn't load Modern Web Guidance here"));
        assert!(t.body.contains("Modern Web Guidance: <id>"));
        // Every package mention is the pinned one
        for (i, _) in t.body.match_indices(&format!("{PACKAGE}@")) {
            assert!(t.body[i..].starts_with(&format!("{PACKAGE}@{VERSION} ")), "unpinned at {i}");
        }
    }

    const WEB_PROMPTS: [&str; 5] = [
        "Add a modal dialog with a form to the settings page",
        "Make the CSS layout responsive on mobile",
        "Fix accessibility of the dropdown menu in the web UI",
        "Add an animation to the popover in the browser",
        "Write the HTML and JavaScript for a dark mode toggle button",
    ];
    const OTHER_PROMPTS: [&str; 3] = [
        "Add a SQL migration for the users table",
        "Fix the Rust compile error in the parser module",
        "Set up the Docker deploy to the VPS with GitHub Actions",
    ];

    #[test]
    fn web_prompts_get_the_skill_and_others_do_not() {
        let skills = vec![as_skill()];
        for p in WEB_PROMPTS {
            assert_eq!(select(&skills, p).len(), 1, "not picked for: {p}");
            assert!(prompt_block_for(&skills, p, false).contains("modern-web-guidance@"), "not injected for: {p}");
        }
        for p in OTHER_PROMPTS {
            assert!(select(&skills, p).is_empty(), "picked for: {p}");
            assert!(!prompt_block_for(&skills, p, false).contains("modern-web-guidance@"), "injected for: {p}");
        }
    }

    fn touch(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn a_web_repo_has_web_code_and_a_rust_one_does_not() {
        let rust = tempfile::tempdir().unwrap();
        for f in ["Cargo.toml", "README.md", "src/main.rs", "src/lib.rs", "tests/a_test.rs", ".github/workflows/ci.yml"] {
            touch(rust.path(), f);
        }
        assert!(!has_web_code(rust.path()));

        for web in ["package.json", "index.html", "web/package.json", "src/App.tsx", "ui/Button.jsx", "app/Page.vue", "lib/Nav.svelte", "static/site.CSS"] {
            let d = tempfile::tempdir().unwrap();
            touch(d.path(), "Cargo.toml");
            touch(d.path(), web);
            assert!(has_web_code(d.path()), "{web}");
        }
    }

    #[test]
    fn deeper_hidden_dependency_and_odd_names_are_not_web_code() {
        let d = tempfile::tempdir().unwrap();
        for f in [
            "a/b/index.html",
            ".hidden/index.html",
            "node_modules/package.json",
            "target/doc.html",
            "node_modules/x/index.css",
            ".css",
            "notes.html.txt",
        ] {
            touch(d.path(), f);
        }
        assert!(!has_web_code(d.path()));
        assert!(!has_web_code(&d.path().join("missing")));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_not_followed() {
        let web = tempfile::tempdir().unwrap();
        touch(web.path(), "index.html");
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(web.path(), d.path().join("linked")).unwrap();
        std::os::unix::fs::symlink(web.path().join("index.html"), d.path().join("page.html")).unwrap();
        assert!(!has_web_code(d.path()));
    }
}
