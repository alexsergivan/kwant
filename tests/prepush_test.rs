//! B2 (0.8.0, fresh-eyes-audit.md §4): before a bot pushes its own branch in a Dev mode space,
//! kwant checks the commit and the push waits for its card. A test switched off blocks the push
//! until the person overrides the card; a clean commit goes once its card is clear. The reviewer
//! turn is another bot's fresh-eyes turn: it sees the diff, never the author's chat.
//!
//! The repo is a throwaway `git init` in a temp dir; nothing touches the network or a remote. The
//! fake claude answers the reviewer with the JSON a `REPLYTEXT:` marker in the diff carries.

mod common;

use base64::Engine;
use common::*;
use kwnt::db::dev_prepush::{BLOCKED, CHECKING, CLEAR, OVERRIDDEN, PUSHED};
use kwnt::drivers::events::RuntimeEvent;
use kwnt::server::dev::prepush;
use serde_json::{json, Value};
use std::path::Path;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Juno (kwant bot)", "-c", "user.email=juno-bot@users.noreply.example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) -> String {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A comment line that makes the fake reviewer answer with a clean review.
fn clean_review(marker: &str) -> String {
    let reply = json!({"intent": {"asked": format!("Count widgets {marker}"), "rows": [{"state": "ok", "text": "Counts the widgets"}]}, "findings": []});
    format!("// {marker}\n// REPLYTEXT:{}\n", base64::engine::general_purpose::STANDARD.encode(reply.to_string()))
}

const TEST_FILE: &str = "tests/widget_test.rs";
const TEST_BODY: &str = "#[test]\nfn counts_widgets() {\n    assert_eq!(2 + 2, 4);\n}\n";

/// A repo on `main` with one test, as the bot's folder.
fn repo(env: &Env, name: &str) -> std::path::PathBuf {
    let folder = env.tmp.path().join("folders").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    commit(&folder, TEST_FILE, TEST_BODY, "Start with a test");
    folder
}

async fn space_of_a(env: &Env) -> String {
    env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id
}

async fn settled(env: &Env, space: &str, branch: &str) -> kwnt::db::dev_prepush::PrepushRow {
    for _ in 0..600 {
        let rows = env.state.db.prepush_cards(space, 50).await.unwrap();
        if let Some(r) = rows.into_iter().find(|r| r.branch == branch && r.status != CHECKING) {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the pre-push card of {branch} never settled");
}

/// The newest card of commit `sha` once it has left `checking`.
async fn settled_sha(env: &Env, space: &str, sha: &str) -> kwnt::db::dev_prepush::PrepushRow {
    for _ in 0..600 {
        let rows = env.state.db.prepush_cards(space, 50).await.unwrap();
        if let Some(r) = rows.into_iter().find(|r| r.head_sha == sha) {
            if r.status != CHECKING {
                return r;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the pre-push card of {sha} never settled");
}

#[tokio::test]
async fn a_switched_off_test_blocks_the_push_until_the_person_overrides() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    let folder = repo(&env, "blocked");
    git(&folder, &["checkout", "-q", "-b", "juno-skip"]);
    let tampered = TEST_BODY.replace("#[test]\n", "#[test]\n#[ignore]\n");
    commit(&folder, "src/widgets.rs", &format!("{}pub fn count() -> u32 {{ 4 }}\n", clean_review("SKIPMARK")), "Count widgets");
    let sha = commit(&folder, TEST_FILE, &tampered, "Skip the slow test");
    let push = "git push -u origin juno-skip";

    // Dev mode off: the push isn't held and no card is made
    assert_eq!(prepush::gate(&env.state, &env.a, push, &folder).await, None);
    assert!(env.state.db.prepush_cards(&space, 50).await.unwrap().is_empty());

    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    // Not a push, or a push to a protected branch (the production floor's card asks the person)
    assert_eq!(prepush::gate(&env.state, &env.a, "git status && git log -1", &folder).await, None);
    assert_eq!(prepush::gate(&env.state, &env.a, "git push origin HEAD:main", &folder).await, None);

    let mut rx = env.state.event_sender.subscribe();
    let first = prepush::gate(&env.state, &env.a, push, &folder).await.expect("the push waits");
    assert!(first.contains("is checking this push of juno-skip") && first.contains("Nothing was pushed"), "{first}");
    let row = settled(&env, &space, "juno-skip").await;
    assert_eq!(row.status, BLOCKED, "{row:?}");
    assert_eq!(row.head_sha, sha);
    assert_eq!(row.kwant_id, env.a);
    let card: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    assert_eq!(card["kind"], "prepush");
    let blockers = card["blockers"].as_array().unwrap();
    assert_eq!(blockers.len(), 1, "only the switched-off test: {card}");
    assert_eq!((blockers[0]["source"].as_str(), blockers[0]["file"].as_str()), (Some("tamper"), Some(TEST_FILE)));
    // The reviewer is another bot, never the author, with fresh eyes
    let reviewer = card["reviewer"]["id"].as_str().expect("a reviewer").to_string();
    assert_ne!(reviewer, env.a, "{card}");
    assert_eq!(card["fresh_eyes"]["checked"], true, "{card}");
    assert_eq!(card["fresh_eyes"]["saw"], json!(["diff"]));
    assert_eq!(card["fresh_eyes"]["reviewer_kwant_id"], json!(reviewer));
    let mut seen = Vec::new();
    while let Ok(e) = rx.try_recv() {
        if let RuntimeEvent::DevPrepush { id, status, .. } = e {
            seen.push((id, status));
        }
    }
    assert_eq!(seen, [(row.id.clone(), "checking".to_string()), (row.id.clone(), "blocked".to_string())]);

    // Still blocked: the bot is told why and how to clear it, and no new card is made
    let again = prepush::gate(&env.state, &env.a, push, &folder).await.expect("still blocked");
    assert!(again.contains("blocked this push") && again.contains(TEST_FILE) && again.contains("override"), "{again}");
    assert_eq!(env.state.db.prepush_cards(&space, 50).await.unwrap().len(), 1);

    // Only the person overrides: a bot's token can't, and a remote call without the token can't
    assert!(call_remote(&env.app, "POST", &format!("/api/dev/prepush/{}/override", row.id)).await.is_client_error());
    let (s, v) = call(&env.app, "GET", &format!("/api/spaces/{space}/dev/prepush"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let api = &v["cards"][0];
    assert_eq!(api["status"], "blocked");
    assert_eq!((api["tries"].as_i64(), api["needs_you"].as_bool()), (Some(1), Some(false)), "the first try is the bot's to fix: {api}");
    assert_eq!(api["diffstat"]["files"], 2, "{api}");
    let state_of = |key: &str| api["checks"].as_array().unwrap().iter().find(|c| c["key"] == key).map(|c| c["state"].as_str().unwrap().to_string());
    let keys: Vec<&str> = api["checks"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["secrets", "pkgs", "tests", "risk", "review", "semgrep"]);
    assert_eq!(state_of("tests").as_deref(), Some("hit"));
    assert_eq!(state_of("review").as_deref(), Some("pass"));
    // gitleaks and the registry are off in tests: a check that couldn't run never shows as passed
    assert_eq!(state_of("secrets").as_deref(), Some("error"));
    assert_eq!(api["blockers"].as_array().unwrap().len(), 1);
    // The person says why, in at least 10 characters
    let url = format!("/api/dev/prepush/{}/override", row.id);
    for body in [None, Some(json!({})), Some(json!({"reason": "   too short  "}))] {
        let (s, v) = call(&env.app, "POST", &url, body).await;
        assert_eq!(s.as_u16(), 400, "{v}");
    }
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"reason": "the slow test is flaky, tracked in #12"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["status"], OVERRIDDEN);
    assert_eq!(v["overridden_by"], "person");
    assert_eq!(v["override"]["reason"], "the slow test is flaky, tracked in #12");
    assert_eq!(v["needs_you"], false);
    // A PR from the branch says it was pushed anyway, with the person's reason
    let anyway = prepush::pushed_anyway(&env.state, &space, Some("juno-skip")).await;
    assert_eq!(anyway[0]["reason"], "the slow test is flaky, tracked in #12", "{anyway}");
    assert_eq!((anyway[0]["head_sha"].as_str(), anyway[0]["by"].as_str()), (Some(sha.as_str()), Some("person")));
    assert_eq!(prepush::pushed_anyway(&env.state, &space, Some("juno-other")).await, json!([]));
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"reason": "the slow test is flaky, tracked in #12"}))).await;
    assert_eq!(s.as_u16(), 409, "an overridden card can't be overridden again");

    // The check alone uses nothing up: a push the person then declines keeps the override
    let pass = prepush::check_command(&env.state, &env.a, push, &folder).await.expect("overridden");
    assert_eq!(pass.overrides, [row.id.clone()]);
    assert_eq!(pass.pinned, None, "`-u` runs as written");
    let racer = prepush::check_command(&env.state, &env.a, push, &folder).await.expect("overridden");
    assert_eq!(env.state.db.prepush_card(&row.id).await.unwrap().unwrap().status, OVERRIDDEN);

    // Now the push goes, once: two parallel calls checked it, and only the first one runs
    assert_eq!(prepush::use_up(&env.state, &pass).await, None);
    let lost = prepush::use_up(&env.state, &racer).await.expect("the override is used up");
    assert!(lost.contains("was for one push") && lost.contains("Nothing was pushed"), "{lost}");
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", row.id), None).await;
    assert_eq!((v["status"].as_str(), v["override_used"].as_bool()), (Some(PUSHED), Some(true)), "{v}");
    assert_eq!(v["override"]["reason"], "the slow test is flaky, tracked in #12");

    // A second push of the same commit gets a blocked card again, until the person overrides again
    let again = prepush::gate(&env.state, &env.a, push, &folder).await.expect("a second push waits for the person");
    assert!(again.contains("was for one push of juno-skip") && again.contains("Nothing was pushed"), "{again}");
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", row.id), None).await;
    assert_eq!((v["id"].as_str(), v["status"].as_str()), (Some(row.id.as_str()), Some(BLOCKED)), "the same card: {v}");
    assert_eq!((v["override"].clone(), v["override_used"].as_bool()), (Value::Null, Some(false)), "{v}");
    assert_eq!(v["last_override"]["reason"], "the slow test is flaky, tracked in #12");
    assert!(prepush::gate(&env.state, &env.a, push, &folder).await.is_some(), "still blocked");
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"reason": "one more push for the demo build"}))).await;
    assert_eq!((s.as_u16(), v["status"].as_str()), (200, Some(OVERRIDDEN)), "{v}");
    assert_eq!(prepush::gate(&env.state, &env.a, push, &folder).await, None);
    assert!(prepush::gate(&env.state, &env.a, push, &folder).await.is_some(), "and that override is one push too");

    // A new commit on the branch is a new check
    let more = commit(&folder, "src/more.rs", "pub fn more() {}\n", "More");
    assert!(prepush::gate(&env.state, &env.a, push, &folder).await.is_some(), "a new commit waits for its own card");
    let second = settled_sha(&env, &space, &more).await;
    assert_eq!((second.status.as_str(), second.tries), (BLOCKED, 1), "after an override the count starts again");
    // The bot tries again without fixing it: the second blocked try in a row needs the person
    let third_sha = commit(&folder, "src/more.rs", "pub fn more() -> u8 { 1 }\n", "More again");
    assert!(prepush::gate(&env.state, &env.a, push, &folder).await.is_some());
    let third = settled_sha(&env, &space, &third_sha).await;
    assert_eq!((third.status.as_str(), third.tries), (BLOCKED, 2));
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", third.id), None).await;
    assert_eq!(v["needs_you"], true, "{v}");
    // The person asks for the checks again: the card is retried and the next push starts at try 1
    assert!(call_remote(&env.app, "POST", &format!("/api/dev/prepush/{}/retry", third.id)).await.is_client_error());
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/prepush/{}/retry", third.id), None).await;
    assert_eq!((s.as_u16(), v["status"].as_str()), (200, Some("retried")), "{v}");
    let (s, _) = call(&env.app, "POST", &format!("/api/dev/prepush/{}/retry", third.id), None).await;
    assert_eq!(s.as_u16(), 409, "only a blocked or failed card is checked again");
    let waits = prepush::gate(&env.state, &env.a, push, &folder).await.expect("checked again");
    assert!(waits.contains("is checking"), "{waits}");
    let fourth = settled_sha(&env, &space, &third_sha).await;
    assert_ne!(fourth.id, third.id, "a new card");
    assert_eq!((fourth.status.as_str(), fourth.tries), (BLOCKED, 1));
}

#[tokio::test]
async fn a_clean_push_goes_once_its_card_is_clear_and_the_reviewer_never_sees_the_authors_chat() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    // The author talked about something private in its chat before the push
    send_direct(&env, &env.a, "Remember AUTHORCHATMARK for later", json!({})).await;
    wait_for("the author's turn", || async { !invocations_with("AUTHORCHATMARK").is_empty() }).await;

    let folder = repo(&env, "clean");
    git(&folder, &["checkout", "-q", "-b", "juno-clean"]);
    let sha = commit(&folder, "src/widgets.rs", &format!("{}pub fn count() -> u32 {{ 4 }}\n", clean_review("CLEANMARK")), "Count widgets");
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();

    let push = "cd . && git push";
    assert!(prepush::gate(&env.state, &env.a, push, &folder).await.is_some(), "the first push waits for the check");
    let row = settled(&env, &space, "juno-clean").await;
    assert_eq!(row.status, CLEAR, "{row:?}");
    assert_eq!(row.head_sha, sha);
    let card: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    assert_eq!(card["blockers"], json!([]), "{card}");
    assert_eq!(card["fresh_eyes"]["checked"], true, "{card}");
    assert_eq!(prepush::gate(&env.state, &env.a, push, &folder).await, None, "a clear card lets the push go");
    assert_eq!(prepush::gate(&env.state, &env.a, push, &folder).await, None, "every time");
    // A push with a remote goes pinned to the commit kwant checked
    let pass = prepush::check_command(&env.state, &env.a, "cd . && git push origin juno-clean", &folder).await.unwrap();
    assert_eq!(pass.pinned, Some(format!("cd . && git push origin {sha}:refs/heads/juno-clean")));
    assert!(pass.overrides.is_empty());

    // The reviewer turn saw the diff and nothing of the author's chat
    let reviews = invocations_with("CLEANMARK");
    assert_eq!(reviews.len(), 1, "one reviewer turn");
    assert!(reviews[0].contains("pub fn count"), "the reviewer sees the diff");
    assert!(!reviews[0].contains("AUTHORCHATMARK"), "the reviewer never gets the author's chat: {}", reviews[0]);
    // A clear card can't be overridden (nothing to let through)
    let (s, _) = call(&env.app, "POST", &format!("/api/dev/prepush/{}/override", row.id), Some(json!({"reason": "nothing to let through here"}))).await;
    assert_eq!(s.as_u16(), 409);
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", row.id), None).await;
    assert!(v["checks"].as_array().unwrap().iter().filter(|c| c["key"] != "semgrep").all(|c| c["state"] != "hit"), "{v}");
    assert_eq!(v["checks"][4]["state"], "pass", "{v}");

    // Nothing new to push: a clear card with no checks and no reviewer turn
    git(&folder, &["checkout", "-q", "-b", "juno-same", "main"]);
    assert!(prepush::gate(&env.state, &env.a, "git push -u origin juno-same", &folder).await.is_some());
    let same = settled(&env, &space, "juno-same").await;
    assert_eq!(same.status, CLEAR, "{same:?}");
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", same.id), None).await;
    assert_eq!((v["no_change"].as_bool(), v["checks"].as_array().map(Vec::len)), (Some(true), Some(0)), "{v}");
    assert_eq!(invocations_with("CLEANMARK").len(), 1, "no reviewer turn for an empty push");
}

#[tokio::test]
async fn pushes_kwant_cant_follow_are_refused_in_dev_mode() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    let folder = repo(&env, "refused");
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    // On a work branch, so a bare push isn't the production floor's (main is protected)
    git(&folder, &["checkout", "-q", "-b", "juno-z"]);
    for cmd in [
        "git -C /tmp/elsewhere push origin x",
        "cd .. && git push origin x",
        "git --git-dir=/tmp/x/.git push origin x",
        // Ada's review of #456: `--repo` writes a branch kwant didn't pin (a matching `:` push is
        // the production floor's card in Dev mode, and refused by the parser: unit tests)
        "git push --repo=origin juno-a:juno-b",
    ] {
        let why = prepush::gate(&env.state, &env.a, cmd, &folder).await;
        assert!(why.is_some(), "{cmd}");
    }
    // A commit in the same command as the push would send a commit nobody checked
    for cmd in ["git commit -am x && git push", "git commit --amend --no-edit; git push -f", "git pull && git push", "bash -c 'git commit -m x && git push'"] {
        let why = prepush::gate(&env.state, &env.a, cmd, &folder).await.expect("refused");
        assert!(why.contains("as its own command"), "{cmd}: {why}");
    }
    // A push from a folder inside the repo but not its top is refused, not let through
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    let why = prepush::gate(&env.state, &env.a, "cd sub && git push origin juno-z", &folder).await.expect("refused");
    assert!(why.contains("top folder"), "{why}");
    assert!(env.state.db.prepush_cards(&space, 50).await.unwrap().is_empty(), "nothing was checked");
}

/// The card rules in the database: one card per (space, folder, branch, commit), a verdict only
/// on a card still checking, an override only from blocked or failed, and a stuck check dropped.
#[tokio::test]
async fn the_card_store_keeps_one_card_per_commit_and_its_rules() {
    let env = setup().await;
    let db = &env.state.db;
    let sha = "a".repeat(40);
    let row = db.insert_prepush_card("spc_x", &env.a, "/f", "juno-x", &sha, CHECKING, None, false).await.unwrap().expect("made");
    assert!(row.id.starts_with("pp_"));
    assert_eq!(row.tries, 1);
    assert_eq!(db.insert_prepush_card("spc_x", &env.a, "/f", "juno-x", &sha, CHECKING, None, false).await.unwrap(), None, "one card per commit");
    assert!(db.insert_prepush_card("spc_x", &env.a, "/f", "juno-y", &sha, CHECKING, None, false).await.unwrap().is_some(), "another branch is another card");
    assert_eq!(db.override_prepush_card(&row.id, "person", "a good reason").await.unwrap(), None, "a card still checking can't be overridden");
    assert_eq!(db.retry_prepush_card(&row.id).await.unwrap(), None, "nor checked again");
    assert!(!db.drop_stuck_prepush_card(&row.id, 60_000).await.unwrap(), "a fresh check isn't stuck");

    let done = db.finish_prepush_card(&row.id, BLOCKED, "{}").await.unwrap().expect("finished");
    assert_eq!(done.status, BLOCKED);
    assert_eq!(db.finish_prepush_card(&row.id, CLEAR, "{}").await.unwrap(), None, "a verdict lands once");
    assert!(!db.drop_stuck_prepush_card(&row.id, -1).await.unwrap(), "only a checking card is dropped");
    // The next commit of a blocked branch is its next try; a retried card starts again at 1
    let b = "b".repeat(40);
    let next = db.insert_prepush_card("spc_x", &env.a, "/f", "juno-x", &b, CHECKING, None, false).await.unwrap().unwrap();
    assert_eq!(next.tries, 2, "one more try after a blocked one");
    assert!(db.insert_prepush_card("spc_x", &env.a, "/g", "juno-x", &b, CHECKING, None, false).await.unwrap().unwrap().tries == 1, "another folder counts apart");
    db.finish_prepush_card(&next.id, BLOCKED, "{}").await.unwrap().unwrap();
    let retried = db.retry_prepush_card(&next.id).await.unwrap().expect("retried");
    assert_eq!(retried.status, "retried");
    assert!(db.drop_retried_prepush_card(&next.id).await.unwrap());
    let fresh = db.insert_prepush_card("spc_x", &env.a, "/f", "juno-x", &b, CHECKING, None, true).await.unwrap().unwrap();
    assert_eq!(fresh.tries, 1, "the person's re-check starts again");
    assert!(!db.drop_retried_prepush_card(&fresh.id).await.unwrap(), "only a retried card is dropped");
    db.finish_prepush_card(&fresh.id, CLEAR, "{}").await.unwrap().unwrap();

    let over = db.override_prepush_card(&row.id, "person", "the slow test is flaky").await.unwrap().expect("overridden");
    assert_eq!((over.status.as_str(), over.overridden_by.as_deref()), (OVERRIDDEN, Some("person")));
    assert_eq!(over.override_reason.as_deref(), Some("the slow test is flaky"));
    assert!(over.overridden_at.is_some());

    let stuck = db.insert_prepush_card("spc_x", &env.a, "/f", "juno-z", &sha, CHECKING, None, false).await.unwrap().unwrap();
    assert!(db.drop_stuck_prepush_card(&stuck.id, -1).await.unwrap(), "a check older than the cutoff is dropped");
    assert_eq!(db.prepush_card(&stuck.id).await.unwrap(), None);
    assert_eq!(db.prepush_cards("spc_x", 50).await.unwrap().len(), 4);
}

/// Ada's review of #456, Later 6: a reviewer turn that didn't answer runs once more on its own;
/// when that fails too, the card waits for the person, saying it was tried again.
#[tokio::test]
async fn a_review_that_errored_runs_once_more_on_its_own() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    let folder = repo(&env, "errored");
    git(&folder, &["checkout", "-q", "-b", "juno-err"]);
    commit(&folder, "src/widgets.rs", "// FAILTURN RETRYMARK\npub fn count() -> u32 { 4 }\n", "Count widgets");
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();

    assert!(prepush::gate(&env.state, &env.a, "git push origin juno-err", &folder).await.is_some());
    let row = settled(&env, &space, "juno-err").await;
    assert_eq!(row.status, BLOCKED, "{row:?}");
    let card: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    assert_eq!(card["auto_retried"], true, "{card}");
    assert!(card["blockers"].as_array().unwrap().iter().any(|b| b["source"] == "review"), "{card}");
    let turns = invocations_with("RETRYMARK").len();
    assert!(turns >= 2, "the reviewer was asked twice: {turns}");
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/prepush/{}", row.id), None).await;
    assert_eq!(v["auto_retried"], true, "{v}");
    // Not a third time on its own: the next push of the same commit waits for the person
    assert!(prepush::gate(&env.state, &env.a, "git push origin juno-err", &folder).await.is_some());
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(invocations_with("RETRYMARK").len(), turns, "no more reviewer turns");
}

/// Rigel's QA of #456, finding C: an override is one push, used up once, never twice.
#[tokio::test]
async fn an_override_is_used_up_by_one_push_in_the_store() {
    let env = setup().await;
    let db = &env.state.db;
    let sha = "b".repeat(40);
    let row = db.insert_prepush_card("spc_o", &env.a, "/f", "juno-o", &sha, CHECKING, None, false).await.unwrap().unwrap();
    assert!(!db.use_prepush_override(&row.id).await.unwrap(), "nothing to use while checking");
    db.finish_prepush_card(&row.id, BLOCKED, "{}").await.unwrap();
    assert!(!db.use_prepush_override(&row.id).await.unwrap(), "nor while blocked");
    assert_eq!(db.reblock_pushed_prepush_card(&row.id).await.unwrap(), None, "only a pushed card is blocked again");
    db.override_prepush_card(&row.id, "person", "a good reason here").await.unwrap().unwrap();
    assert!(db.use_prepush_override(&row.id).await.unwrap());
    assert!(!db.use_prepush_override(&row.id).await.unwrap(), "once");
    assert_eq!(db.prepush_card(&row.id).await.unwrap().unwrap().status, PUSHED);
    assert_eq!(db.override_prepush_card(&row.id, "person", "another good reason").await.unwrap(), None, "a used override isn't overridden again until blocked");
    let again = db.reblock_pushed_prepush_card(&row.id).await.unwrap().unwrap();
    assert_eq!((again.status.as_str(), again.override_reason.as_deref()), (BLOCKED, Some("a good reason here")), "the history stays");
    assert_eq!(db.prepush_overrides_on("spc_o", "juno-o", 3).await.unwrap().len(), 1);
    assert!(db.prepush_overrides_on("spc_o", "juno-p", 3).await.unwrap().is_empty());
    // Blocked again only because its one push went: the next commit's count starts again
    let next = db.insert_prepush_card("spc_o", &env.a, "/f", "juno-o", &"c".repeat(40), CHECKING, None, false).await.unwrap().unwrap();
    assert_eq!(next.tries, 1);
    db.finish_prepush_card(&next.id, BLOCKED, "{}").await.unwrap();
    let after = db.insert_prepush_card("spc_o", &env.a, "/f", "juno-o", &"d".repeat(40), CHECKING, None, false).await.unwrap().unwrap();
    assert_eq!(after.tries, 2, "a blocked card nobody overrode still counts");
}

/// A GitHub tool's write (`create_or_update_file`) to `branch`, with `files` as `(path, text)`.
fn gh_write(branch: Option<&str>, path: &str, text: &str) -> Value {
    let mut v = json!({"owner": "demo-org", "repo": "demo-app", "path": path, "content": text, "message": "Count widgets"});
    if let Some(b) = branch {
        v["branch"] = json!(b);
    }
    v
}

#[tokio::test]
async fn kwants_own_github_tools_go_through_the_pre_push_card() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    let tampered = format!("{}{}", clean_review("GHWRITEMARK"), TEST_BODY.replace("#[test]\n", "#[test]\n#[ignore]\n"));
    let args = gh_write(Some("juno-gh"), TEST_FILE, &tampered);

    // Dev mode off: the write isn't held and no card is made
    assert_eq!(prepush::check_mcp(&env.state, &env.a, "github", "create_or_update_file", &args).await, Ok(prepush::Pass::default()));
    assert!(env.state.db.prepush_cards(&space, 50).await.unwrap().is_empty());
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();

    // Reads, kwant's own tools and tools that name no repo aren't writes
    for (server, tool, a) in [
        ("github", "get_file_contents", json!({"owner": "demo-org", "repo": "demo-app", "path": "README.md"})),
        ("kwnt_tools", "create_or_update_file", args.clone()),
        ("files", "delete_file", json!({"path": "notes.txt"})),
    ] {
        assert_eq!(prepush::check_mcp(&env.state, &env.a, server, tool, &a).await, Ok(prepush::Pass::default()), "{tool}");
    }
    assert!(env.state.db.prepush_cards(&space, 50).await.unwrap().is_empty());
    // A write that names no branch would land on the default branch: refused without a card
    let why = prepush::check_mcp(&env.state, &env.a, "github", "create_or_update_file", &gh_write(None, "a.txt", "hi")).await.unwrap_err();
    assert!(why.contains("names no branch"), "{why}");
    // Composio's multi-execute with two writes: one call each
    let multi = json!({"tools": [
        {"tool_slug": "GITHUB_CREATE_OR_UPDATE_FILE_CONTENTS", "arguments": gh_write(Some("juno-gh"), "a.txt", "a")},
        {"tool_slug": "GITHUB_DELETE_A_FILE", "arguments": gh_write(Some("juno-gh"), "b.txt", "")},
    ]});
    let why = prepush::check_mcp(&env.state, &env.a, "composio", "COMPOSIO_MULTI_EXECUTE_TOOL", &multi).await.unwrap_err();
    assert!(why.contains("makes 2 GitHub writes"), "{why}");

    // The write waits for its card, which the switched-off test blocks
    let first = prepush::check_mcp(&env.state, &env.a, "github", "create_or_update_file", &args).await.unwrap_err();
    assert!(first.contains("is checking this push of juno-gh") && first.contains("GitHub tool's write"), "{first}");
    let row = settled(&env, &space, "juno-gh").await;
    assert_eq!(row.status, BLOCKED, "{row:?}");
    assert_eq!(row.folder, "mcp:github/demo-org/demo-app");
    assert_eq!(row.head_sha, prepush::write_id("create_or_update_file", &args));
    let card: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    assert_eq!(card["mcp"], json!({"owner": "demo-org", "repo": "demo-app"}), "{card}");
    assert_eq!(card["blockers"].as_array().unwrap().iter().map(|b| b["source"].as_str().unwrap()).collect::<Vec<_>>(), ["tamper"], "{card}");
    assert_eq!(card["fresh_eyes"]["checked"], true, "{card}");
    assert_eq!(card["security"]["semgrep"]["state"], "not_run", "{card}");
    let reviews = invocations_with("GHWRITEMARK");
    assert_eq!(reviews.len(), 1, "one reviewer turn");
    assert!(reviews[0].contains("counts_widgets"), "the reviewer sees what the tool sends");
    // Still blocked, and the same card
    assert!(prepush::check_mcp(&env.state, &env.a, "github", "create_or_update_file", &args).await.unwrap_err().contains("blocked this push"));
    // A different write is a different card
    assert_eq!(env.state.db.prepush_cards(&space, 50).await.unwrap().len(), 1);

    // claude asks kwant's permission prompt before calling the GitHub server itself: denied there
    let mut github = kwnt::db::models::McpServer {
        id: "id-github".into(),
        name: "github".into(),
        transport: "stdio".into(),
        command: Some("/usr/bin/true".into()),
        args: vec![],
        url: None,
        env: Default::default(),
        headers: Default::default(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    };
    github.enabled = true;
    let ctx = kwnt::mcp::McpTurnContext {
        servers: vec![github],
        approver: std::sync::Arc::new(kwnt::mcp::FixedApprover(true)),
        config_dir: env.tmp.path().join("mcp-gh"),
        permission_url: None,
        gateway: None,
        max_tool_iterations: 8,
        smart_approvals: false,
        records: Default::default(),
    };
    let session = kwnt::server::mcp_runtime::PermissionSession::new(env.a.clone(), "A".into(), "gh-turn".into(), ctx, None);
    env.state.mcp_sessions.lock().await.insert("gh-tok".into(), session);
    let ask = |input: &Value| json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "approve", "arguments": {"tool_name": "mcp__github__create_or_update_file", "input": input, "tool_use_id": "tu_gh"}}});
    let decision = |v: &Value| -> Value { serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap() };
    let (s, v) = call(&env.app, "POST", "/api/mcp/permission/gh-tok", Some(ask(&args))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let d = decision(&v);
    assert_eq!(d["behavior"], "deny", "{d}");
    assert!(d["message"].as_str().unwrap().contains("blocked this push"), "{d}");

    // The person overrides: exactly one call goes, through the turn's approver
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/prepush/{}/override", row.id), Some(json!({"reason": "the test is flaky on CI, fixing next"}))).await;
    assert_eq!((s.as_u16(), v["status"].as_str()), (200, Some(OVERRIDDEN)), "{v}");
    let approver = kwnt::server::mcp_runtime::PrepushApprover { inner: std::sync::Arc::new(kwnt::mcp::FixedApprover(true)), state: env.state.clone() };
    use kwnt::mcp::ToolApprover;
    let req = kwnt::mcp::ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "gh-turn".into(),
        tool_name: "mcp__github__create_or_update_file".into(),
        server: "github".into(),
        tool: "create_or_update_file".into(),
        arguments: args.clone(),
        builtin: None,
        smart: None,
        payment: None,
    };
    let overrides = approver.prepush_gate(&req).await.expect("the override lets it go");
    assert_eq!(overrides, [row.id.clone()]);
    assert_eq!(approver.prepush_use(&overrides).await, None);
    assert_eq!(env.state.db.prepush_card(&row.id).await.unwrap().unwrap().status, PUSHED);
    // The same write again waits for the person again, on the permission prompt too
    let d = decision(&call(&env.app, "POST", "/api/mcp/permission/gh-tok", Some(ask(&args))).await.1);
    assert_eq!(d["behavior"], "deny", "{d}");
    assert!(d["message"].as_str().unwrap().contains("was for one push"), "{d}");
    assert_eq!(env.state.db.prepush_card(&row.id).await.unwrap().unwrap().status, BLOCKED);
    // The reason goes with the PR's review
    let anyway = prepush::pushed_anyway(&env.state, &space, Some("juno-gh")).await;
    assert_eq!(anyway[0]["reason"], "the test is flaky on CI, fixing next", "{anyway}");
}
