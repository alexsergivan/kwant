//! Test plan #20 (D4, S-12): a folder an isolated Claude turn used may hold settings, hooks or
//! scripts a bot wrote from inside the VM. On this computer Codex and Grok never start in it
//! (equal, inside or holding it) and Claude starts with only the person's own settings.
#![cfg(unix)]

#[path = "common/exe.rs"]
mod exe;

use kwnt::{
    db::{models::Kwant, Database},
    drivers::{CliDriver, Driver, TurnInput, TurnOutput},
};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

fn turn(store: &Database, engine: &str, folder: &Path) -> TurnInput {
    TurnInput {
        fresh_eyes: false,
        expects: String::new(),
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: Some(store.clone()),
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Ada".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: engine.into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "hi".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: Some(folder.to_path_buf()),
        sandbox_id: None,
        teammates: vec![],
        mcp: None,
        no_tools: false,
        read_only: false,
        delivery: None,
    }
}

/// A fake CLI that leaves a marker (any start at all, `mcp list` included) and its argv.
struct Fake {
    dir: TempDir,
}

impl Fake {
    fn new(engine: &str) -> Self {
        let dir = tempdir().unwrap();
        let o = dir.path().display();
        exe::write_executable(dir.path().join(engine), format!("#!/bin/sh\necho started >> {o}/marker\nprintf '%s\\n' \"$@\" >> {o}/argv\n"));
        Self { dir }
    }

    fn started(&self) -> bool {
        self.dir.path().join("marker").exists()
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("argv")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn reset(&self) {
        let _ = std::fs::remove_file(self.dir.path().join("marker"));
        let _ = std::fs::remove_file(self.dir.path().join("argv"));
    }

    async fn run(&self, store: &Database, engine: &str, folder: &Path) -> TurnOutput {
        let bin = self.dir.path().join(engine).to_string_lossy().into_owned();
        let mut driver = CliDriver::new(Some(bin), Some(engine.into()));
        if engine == "grok" {
            let home = self.dir.path().join("grok-home");
            std::fs::create_dir_all(&home).unwrap();
            driver = driver.with_grok_home(home);
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let out = driver.stream_turn(turn(store, engine, folder), tx).await.unwrap();
        drain.await.unwrap();
        out
    }
}

struct Env {
    tmp: TempDir,
    db: Database,
    recorded: PathBuf,
    other: PathBuf,
}

async fn env() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(&tmp.path().join("data")).await.unwrap();
    let recorded = tmp.path().join("projects").join("site");
    let other = tmp.path().join("elsewhere");
    for d in [recorded.join("sub"), other.clone()] {
        std::fs::create_dir_all(d).unwrap();
    }
    let recorded = recorded.canonicalize().unwrap();
    db.record_isolated_folder(&recorded, "isolated-bot").await.unwrap();
    Env { tmp, db, recorded, other: other.canonicalize().unwrap() }
}

fn refusal(out: &TurnOutput, engine: &str) {
    let (text, meta) = out.notice.as_ref().expect("a notice, not a reply");
    assert!(!text.is_empty());
    assert_eq!(meta["reason"], "folder_used_by_isolated_bot", "{meta}");
    assert_eq!(meta["engine"], engine, "{meta}");
}

#[tokio::test]
async fn codex_and_grok_never_start_in_a_folder_an_isolated_bot_used() {
    let e = env().await;
    let parent = e.recorded.parent().unwrap().to_path_buf();
    for engine in ["codex", "grok"] {
        let fake = Fake::new(engine);
        // The folder itself, a subfolder of it, and a folder holding it
        for folder in [e.recorded.clone(), e.recorded.join("sub"), parent.clone()] {
            let out = fake.run(&e.db, engine, &folder).await;
            refusal(&out, engine);
            assert!(!fake.started(), "{engine} started in {}", folder.display());
        }
    }
    // Positive control: Codex in a folder no isolated bot used starts as usual
    let fake = Fake::new("codex");
    let out = fake.run(&e.db, "codex", &e.other).await;
    assert!(out.notice.is_none(), "{:?}", out.notice);
    assert!(fake.started());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn host_claude_in_a_used_folder_loads_only_the_persons_settings() {
    let e = env().await;
    let fake = Fake::new("claude");
    for folder in [e.recorded.clone(), e.recorded.join("sub")] {
        fake.reset();
        let out = fake.run(&e.db, "claude", &folder).await;
        assert!(out.notice.is_none(), "{:?}", out.notice);
        let argv = fake.argv();
        let at = argv.iter().position(|a| a == "--setting-sources").unwrap_or_else(|| panic!("{argv:?}"));
        assert_eq!(argv[at + 1], "user");
        assert!(argv.iter().any(|a| a == "--strict-mcp-config"), "{argv:?}");
    }
    // Positive control: elsewhere Claude's project settings load as before
    fake.reset();
    fake.run(&e.db, "claude", &e.other).await;
    let argv = fake.argv();
    assert!(argv.iter().any(|a| a == "--strict-mcp-config"), "{argv:?}");
    assert!(!argv.iter().any(|a| a == "--setting-sources"), "{argv:?}");
}

#[tokio::test]
async fn a_folder_kwnt_cant_check_counts_as_used() {
    let e = env().await;
    // The record can't be read (another connection renames its table): the guard fails closed
    let file = e.tmp.path().join("data").join("kwnt.db");
    let other = sqlx::SqlitePool::connect(&format!("sqlite://{}", file.display())).await.unwrap();
    sqlx::query("ALTER TABLE isolated_folders RENAME TO isolated_folders_gone").execute(&other).await.unwrap();
    other.close().await;
    let fake = Fake::new("codex");
    let out = fake.run(&e.db, "codex", &e.other).await;
    refusal(&out, "codex");
    assert!(!fake.started());
}
