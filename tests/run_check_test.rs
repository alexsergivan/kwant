//! `kwnt run-check` against a temp database with the real schema: the binary is run the way a
//! person would, and its words, JSON and exit code are checked.

use kwnt::db::Database;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

const FORGE: &str = "2ffd9c25-176b-4914-9e71-a7079f349bd1";
const PEBBLE: &str = "79ec92e0-7996-4240-90b9-84e2c7db5593";

/// A database made by kwnt itself (every migration run), with two bots.
async fn fresh(dir: &Path) {
    Database::init(dir).await.unwrap();
    exec(
        dir,
        &format!(
            "INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite) VALUES ('{FORGE}', 'Forge', 'Dev', 'p', 'a');
             INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite) VALUES ('{PEBBLE}', 'Pebble', 'Ops', 'p', 'a')"
        ),
    )
    .await;
}

async fn exec(dir: &Path, sql: &str) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rw", dir.join("kwnt.db").display()))
        .await
        .unwrap();
    for stmt in sql.split(";\n").map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt).execute(&pool).await.unwrap_or_else(|e| panic!("{stmt}: {e}"));
    }
    pool.close().await;
}

/// A person message in `bot`'s direct chat at `at` (`2026-11-06 18:30:23`).
async fn person(dir: &Path, id: &str, bot: &str, at: &str) {
    exec(
        dir,
        &format!("INSERT INTO messages (id, direct_kwant_id, sender_role, sender_id, text, created_at) VALUES ('{id}', '{bot}', 'user', 'you', 'hi', '{at}')"),
    )
    .await;
}

/// A tool step that ran at `at` (RFC 3339).
fn step(tool: &str, at: &str, title: &str, detail: Option<&str>) -> Value {
    let mut s = json!({"id": "s1", "kind": "tool", "status": "ok", "tool": tool, "title": title, "started_at": at, "ended_at": at, "duration_ms": 50});
    if let Some(d) = detail {
        s["detail"] = json!(d);
    }
    s
}

/// A claude reply by `name` in `bot`'s direct chat, saved at `at`.
async fn reply(dir: &Path, id: &str, bot: &str, name: &str, at: &str, steps: Vec<Value>) {
    let meta = json!({"activity": {"steps": steps}, "ran_on": {"engine": "claude"}});
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rw", dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query("INSERT INTO messages (id, direct_kwant_id, sender_role, sender_id, text, created_at, meta) VALUES (?, ?, 'kwant', ?, 'done', ?, ?)")
        .bind(id)
        .bind(bot)
        .bind(name)
        .bind(at)
        .bind(meta.to_string())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// An approvals row for `bot`'s direct chat.
async fn card(dir: &Path, id: &str, bot: &str, tool: &str, command: &str, status: &str, at: &str) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rw", dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query("INSERT INTO approvals (id, turn_id, kwant_id, tool_name, parameters, status, created_at, resume) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id)
        .bind(format!("turn-{id}"))
        .bind(bot)
        .bind(tool)
        .bind(json!({"arguments": {"command": command}}).to_string())
        .bind(status)
        .bind(at)
        .bind(json!({"agent": "claude", "direct_kwant_id": bot}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run_check(dir: &Path, args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_kwnt"))
        .arg("--data-dir")
        .arg(dir)
        .arg("run-check")
        .args(args)
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Forge ran `gh pr merge 7` with no card, then a clean carded `gh pr view`.
async fn one_uncarded_merge(dir: &Path) {
    fresh(dir).await;
    person(dir, "p1", FORGE, "2026-11-06 18:30:00").await;
    reply(
        dir,
        "r1",
        FORGE,
        "Forge",
        "2026-11-06 18:30:09",
        vec![
            step("Bash", "2026-11-06T18:30:02.100Z", "Running `gh pr view 7`", Some("gh pr view 7")),
            step("Bash", "2026-11-06T18:30:05.400Z", "Running `gh pr merge 7 --squash`", Some("gh pr merge 7 --squash")),
        ],
    )
    .await;
    card(dir, "a1", FORGE, "Bash", "gh pr view 7", "allowed_once", "2026-11-06 18:30:02").await;
}

#[tokio::test]
async fn a_step_that_ran_with_no_card_is_flagged() {
    let tmp = tempdir().unwrap();
    one_uncarded_merge(tmp.path()).await;
    let run = run_check(tmp.path(), &["--since", "all"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    assert!(run.out.starts_with("Checked 1 reply since 2026-11-06 18:30:09 UTC: 2 tool steps, 1 with a card.\n"), "{}", run.out);
    assert!(
        run.out.contains("Ran with no card (1)\n  2026-11-06 18:30:05 UTC · Forge · chat with Forge · Bash · Running `gh pr merge 7 --squash`\n"),
        "{}",
        run.out
    );
    assert!(!run.out.contains("gh pr view"), "{}", run.out);
}

#[tokio::test]
async fn a_clean_chat_has_nothing_to_flag() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    fresh(dir).await;
    person(dir, "p1", PEBBLE, "2026-11-06 18:28:13").await;
    reply(
        dir,
        "r1",
        PEBBLE,
        "Pebble",
        "2026-11-06 18:28:40",
        vec![
            step("Bash", "2026-11-06T18:28:20.000Z", "Running `ls -la`", Some("ls -la")),
            step("Write", "2026-11-06T18:28:24.478Z", "Writing …/79ec92e0/tareas.txt", None),
            step("Bash", "2026-11-06T18:28:29.300Z", "Running `rm -f old.txt`", Some("rm -f old.txt")),
            step("mcp__kwnt_tools__ask_user", "2026-11-06T18:28:35.000Z", "Asking you", None),
        ],
    )
    .await;
    card(dir, "a1", PEBBLE, "Bash", "rm -f old.txt", "allowed_once", "2026-11-06 18:28:29").await;
    let run = run_check(dir, &["--since", "all"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    assert!(run.out.ends_with("Nothing to flag since 2026-11-06 18:28:40 UTC.\n"), "{}", run.out);
    assert!(run.out.contains("4 tool steps, 1 with a card."), "{}", run.out);
    assert!(run.out.contains("No card needed: 1 file read or edit in the bot's own folder, 1 read-only command, 1 kwant tool."), "{}", run.out);
}

#[tokio::test]
async fn a_later_call_of_an_always_allowed_tool_is_not_flagged() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    fresh(dir).await;
    reply(dir, "r1", FORGE, "Forge", "2026-11-06 18:30:09", vec![step("mcp__github__merge_pr", "2026-11-06T18:30:02.100Z", "Merging #7", None)]).await;
    card(dir, "a1", FORGE, "mcp__github__merge_pr", "", "always_allowed", "2026-11-06 18:30:02").await;
    // An hour later: claude asks again, kwnt's session "Always allow" answers with no row
    reply(dir, "r2", FORGE, "Forge", "2026-11-06 19:31:00", vec![step("mcp__github__merge_pr", "2026-11-06T19:30:55.000Z", "Merging #8", None)]).await;
    let run = run_check(dir, &["--since", "all", "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    let v: Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(v["findings"], json!([]), "{v}");
    assert_eq!(v["with_card"], 1);
    assert_eq!(v["by_design"]["always_allowed"], 1);

    // The same call by another bot, or in another chat, isn't covered by Forge's choice
    reply(dir, "r3", PEBBLE, "Pebble", "2026-11-06 19:40:00", vec![step("mcp__github__merge_pr", "2026-11-06T19:39:55.000Z", "Merging #9", None)]).await;
    let v: Value = serde_json::from_str(&run_check(dir, &["--since", "all", "--json"]).out).unwrap();
    assert_eq!(v["findings"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["findings"][0]["bot"], "Pebble");
}

#[tokio::test]
async fn strict_exits_1_with_findings_and_0_without() {
    let tmp = tempdir().unwrap();
    one_uncarded_merge(tmp.path()).await;
    let run = run_check(tmp.path(), &["--since", "all", "--strict"]);
    assert_eq!(run.code, 1, "{}{}", run.out, run.err);
    assert!(run.out.contains("Ran with no card (1)"), "{}", run.out);
    // Only Pebble, who ran nothing
    let run = run_check(tmp.path(), &["--since", "all", "--strict", "--bot", "pebble"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    assert!(run.out.contains("Nothing to flag since"), "{}", run.out);
    // An empty database
    let empty = tempdir().unwrap();
    fresh(empty.path()).await;
    let run = run_check(empty.path(), &["--strict"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    assert!(run.out.contains("Nothing to flag since"), "{}", run.out);
}

#[tokio::test]
async fn a_secret_in_a_command_is_redacted() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    fresh(dir).await;
    let token = format!("sk-{}", "Q7x".repeat(12));
    let command = format!("curl -H 'x-api-key: {token}' -d password=hunter2hunter2 https://api.example.com/charge");
    reply(dir, "r1", FORGE, "Forge", "2026-11-06 18:30:09", vec![step("Bash", "2026-11-06T18:30:02.100Z", &format!("Running `{command}`"), Some(&command))]).await;
    for args in [&["--since", "all"][..], &["--since", "all", "--json"][..]] {
        let run = run_check(dir, args);
        assert_eq!(run.code, 0, "{}{}", run.out, run.err);
        assert!(run.out.contains("curl"), "{}", run.out);
        assert!(!run.out.contains(&token), "{}", run.out);
        assert!(!run.out.contains("hunter2hunter2"), "{}", run.out);
    }
}

#[tokio::test]
async fn it_reads_and_never_writes() {
    let tmp = tempdir().unwrap();
    one_uncarded_merge(tmp.path()).await;
    let db = tmp.path().join("kwnt.db");
    let before = std::fs::read(&db).unwrap();
    let run = run_check(tmp.path(), &["--since", "all", "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.out, run.err);
    assert_eq!(std::fs::read(&db).unwrap(), before, "kwnt.db changed");
    assert!(!tmp.path().join("kwnt.db-wal").exists() || std::fs::metadata(tmp.path().join("kwnt.db-wal")).unwrap().len() == 0);
}

#[tokio::test]
async fn bad_input_exits_2_and_says_why() {
    let tmp = tempdir().unwrap();
    one_uncarded_merge(tmp.path()).await;
    let run = run_check(tmp.path(), &["--bot", "Nobody"]);
    assert_eq!(run.code, 2);
    assert!(run.err.contains("no bot named Nobody"), "{}", run.err);
    let run = run_check(tmp.path(), &["--since", "fortnight"]);
    assert_eq!(run.code, 2);
    assert!(run.err.contains("isn't a time span"), "{}", run.err);
    let nowhere = tempdir().unwrap();
    let run = run_check(nowhere.path(), &[]);
    assert_eq!(run.code, 2);
    assert!(run.err.contains("no kwant database"), "{}", run.err);
    assert!(!nowhere.path().join("kwnt.db").exists());
}

#[tokio::test]
async fn since_leaves_out_older_replies() {
    let tmp = tempdir().unwrap();
    one_uncarded_merge(tmp.path()).await;
    // 2026-11-06 is long gone for a 24h window once the test runs later; a fresh reply is in it
    let now = chrono::Utc::now();
    let at = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let step_at = (now - chrono::Duration::seconds(5)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    reply(tmp.path(), "r2", PEBBLE, "Pebble", &at, vec![step("Bash", &step_at, "Running `touch x`", Some("touch x"))]).await;
    let v: Value = serde_json::from_str(&run_check(tmp.path(), &["--since", "24h", "--json"]).out).unwrap();
    assert_eq!(v["replies"], 1, "{v}");
    assert_eq!(v["findings"][0]["bot"], "Pebble");
}
