//! G1 line notes (release-090/g1-api.md): the diff of a review card, notes on its lines, one
//! message to the author bot, and the check on the next push.

mod common;

use common::*;
use kwnt::db::dev_mode::{LANDED, REVIEW, STALE};
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::line_notes;
use serde_json::{json, Value};
use std::time::Duration;

const REPO: &str = "github.com/demo-org/app";

const PATCH: &str = "@@ -38,6 +38,7 @@ export function InboxEmpty\n   const x = 1;\n-  return (\n+  return (\n+    <section aria-label=\"Inbox\">\n   <div>\n   </div>\n@@ -80,3 +81,3 @@\n a\n-b\n+B\n c";

fn facts(head: &str, bot: Option<(&str, &str)>, branch: &str, patch: &str) -> String {
    json!({
        "repo": "demo-org/app", "kind": "review", "pr": 212, "title": "Inbox empty state", "body": "",
        "head_sha": head, "base_branch": "main", "merged_at": null, "merged_by": null,
        "author": {"login": "pixel-bot", "name": "Pixel", "email": null,
                   "bot": bot.map(|(id, name)| json!({"id": id, "name": name, "engine": "claude", "model": "x"}))},
        "files": [
            {"path": "src/InboxEmpty.tsx", "status": "modified", "additions": 3, "deletions": 2, "patch": patch, "previous_path": null},
            {"path": "assets/logo.png", "status": "added", "additions": 0, "deletions": 0, "patch": null, "previous_path": null},
            {"path": "Cargo.lock", "status": "modified", "additions": 900, "deletions": 3, "patch": null, "previous_path": null}
        ],
        "cut": "diff cut, 1 files not read", "checks": [], "ci": "green", "task": null, "first_edit_files": [],
        "head_branch": branch, "links": {"pr": "https://github.com/demo-org/app/pull/212", "commit": null, "ci": null}
    })
    .to_string()
}

async fn review(env: &Env, head: &str, patch: &str) -> kwnt::db::dev_mode::DevCardRow {
    tokio::time::sleep(Duration::from_millis(5)).await;
    let space = SpaceId::default_space();
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    env.state
        .db
        .insert_dev_card_in(space.as_str(), REPO, REVIEW, "pr:212", head, &facts(head, Some((&bot.id, &bot.name)), "pixel/inbox-empty", patch))
        .await
        .unwrap()
        .unwrap()
}

fn note(head: &str, start: Value, end: Value, text: &str) -> Value {
    json!({"path": "src/InboxEmpty.tsx", "start": start, "end": end, "head_sha": head, "text": text})
}

#[tokio::test]
async fn the_diff_is_the_card_s_own_files_as_hunks() {
    let env = setup().await;
    let card = review(&env, "aaa1111", PATCH).await;
    let (s, v) = call(&env.app, "GET", &format!("/api/dev/cards/{}/changes", card.id), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["pr"].clone(), v["head_sha"].clone(), v["pr_state"].clone()), (json!(212), json!("aaa1111"), json!("open")));
    assert_eq!(v["author_bot"]["id"], env.a.as_str());
    assert_eq!(v["cut"], json!({"files_not_read": 1}));
    let f = &v["files"];
    assert_eq!(f[0]["hunks"][0]["lines"][1], json!({"kind": "del", "old": 39, "new": null, "text": "  return ("}));
    assert_eq!(f[0]["hunks"][1]["header"], "@@ -80,3 +81,3 @@");
    assert_eq!((f[0]["omitted"].clone(), f[0]["big"].clone()), (Value::Null, json!(false)));
    assert_eq!((f[1]["omitted"].clone(), f[1]["hunks"].clone()), (json!("binary"), json!([])));
    assert_eq!((f[2]["omitted"].clone(), f[2]["big"].clone()), (json!("too_large"), json!(true)));

    // The card payload carries the stat, the head and (no notes yet) null counts
    let (_, c) = call(&env.app, "GET", &format!("/api/dev/cards/{}", card.id), None).await;
    assert_eq!(c["changes"], json!({"files": 3, "additions": 903, "deletions": 5}), "{c}");
    assert_eq!((c["head_sha"].clone(), c["notes"].clone()), (json!("aaa1111"), Value::Null));

    // Only a review card has a diff; agents are refused
    let (s, v) = call(&env.app, "GET", "/api/dev/cards/dvc_nope/changes", None).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (404, json!("not_found")));
    let (s, _) = call_with(&env.app, "GET", &format!("/api/dev/cards/{}/changes", card.id), None, &[("authorization", "Bearer not-the-person")]).await;
    assert_eq!(s.as_u16(), 403);
}

#[tokio::test]
async fn notes_are_written_edited_deleted_and_restored() {
    let env = setup().await;
    let card = review(&env, "aaa1111", PATCH).await;
    let notes = format!("/api/dev/cards/{}/notes", card.id);
    let new = |l: u32| json!({"side": "new", "line": l});
    let old = |l: u32| json!({"side": "old", "line": l});

    // A mixed range, dragged upwards: the removed line and both added lines
    let (s, n) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(40), old(39), "Use aria-labelledby."))).await;
    assert_eq!(s.as_u16(), 201, "{n}");
    assert_eq!((n["start"].clone(), n["end"].clone()), (old(39), new(40)), "ends in diff order");
    assert_eq!(n["code"].as_array().unwrap().iter().map(|l| l["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["del", "add", "add"]);
    assert_eq!((n["state"].clone(), n["at_head"]["start"].clone(), n["seen_sha"].clone()), (json!("draft"), old(39), Value::Null));
    let id = n["id"].as_str().unwrap().to_string();

    // Refusals, each with its code
    for (body, status, code) in [
        (note("bbb", new(40), new(40), "x"), 409, "stale_head"),
        (note("aaa1111", new(38), new(82), "x"), 422, "cross_hunk"),
        (note("aaa1111", new(500), new(500), "x"), 422, "not_in_diff"),
        (note("aaa1111", new(40), new(40), "   "), 422, "empty"),
        (note("aaa1111", new(40), new(40), &"x".repeat(4001)), 422, "too_long"),
        (json!({"text": "no path"}), 400, "bad_request"),
    ] {
        let (s, v) = call(&env.app, "POST", &notes, Some(body)).await;
        assert_eq!((s.as_u16(), v["code"].as_str().unwrap()), (status, code), "{v}");
        assert!(v["error"].is_string());
        if code == "stale_head" {
            assert_eq!(v["head_sha"], "aaa1111");
        }
    }

    // Edit, then the counts on the card
    let (s, e) = call(&env.app, "PATCH", &format!("/api/dev/notes/{id}"), Some(json!({"text": "Use aria-labelledby, please."}))).await;
    assert_eq!((s.as_u16(), e["text"].clone()), (200, json!("Use aria-labelledby, please.")));
    let (s, _) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(82), new(82), "Why B?"))).await;
    assert_eq!(s.as_u16(), 201);
    let (_, list) = call(&env.app, "GET", &format!("/api/spaces/{}/dev/reviews", SpaceId::default_space().as_str()), None).await;
    assert_eq!(list["cards"][0]["notes"], json!({"drafts": 2, "total": 2}), "{list}");

    // Delete and restore within 60 s; a deleted note isn't listed or counted
    let (s, d) = call(&env.app, "DELETE", &format!("/api/dev/notes/{id}"), None).await;
    assert_eq!((s.as_u16(), d), (200, json!({"id": id, "deleted": true})));
    let (_, l) = call(&env.app, "GET", &notes, None).await;
    assert_eq!(l["notes"].as_array().unwrap().len(), 1);
    let (_, c) = call(&env.app, "GET", &format!("/api/dev/cards/{}", card.id), None).await;
    assert_eq!(c["notes"], json!({"drafts": 1, "total": 1}));
    let (s, r) = call(&env.app, "POST", &format!("/api/dev/notes/{id}/restore"), None).await;
    assert_eq!((s.as_u16(), r["id"].clone()), (200, json!(id)));
    let (_, l) = call(&env.app, "GET", &notes, None).await;
    assert_eq!(l["notes"].as_array().unwrap().len(), 2);

    // Discard all drafts
    let (s, v) = call(&env.app, "DELETE", &notes, None).await;
    assert_eq!((s.as_u16(), v["discarded"].clone()), (200, json!(2)));
    let (_, l) = call(&env.app, "GET", &notes, None).await;
    assert_eq!(l["notes"], json!([]));
    let (s, v) = call(&env.app, "PATCH", "/api/dev/notes/ln_nope", Some(json!({"text": "x"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (404, json!("not_found")));
}

#[tokio::test]
async fn send_posts_one_message_to_the_author_bot_once() {
    let env = setup().await;
    let card = review(&env, "aaa1111", PATCH).await;
    let notes = format!("/api/dev/cards/{}/notes", card.id);
    let new = |l: u32| json!({"side": "new", "line": l});
    let (_, a) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(40), new(41), "Use aria-labelledby."))).await;
    let (_, b) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(82), new(82), "Why B?"))).await;
    let send = format!("{notes}/send");

    let (s, v) = call(&env.app, "POST", &send, Some(json!({"send_id": "s-1"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["sent"].clone(), v["kwant_id"].clone()), (json!(2), json!(env.a)));
    assert!(v["notes"].as_array().unwrap().iter().all(|n| n["state"] == "sent" && n["message_id"] == v["message_id"]), "{v}");

    // One message from the person in the bot's 1:1, Mira's block, meta on the message object
    let (_, msgs) = call(&env.app, "GET", &format!("/api/channels/{}/messages", env.a), None).await;
    let mine: Vec<&Value> = msgs.as_array().unwrap().iter().filter(|m| m["text"].as_str().is_some_and(|t| t.contains("from the review"))).collect();
    assert_eq!(mine.len(), 1, "{msgs}");
    let m = mine[0];
    assert_eq!((m["sender_role"].clone(), m["id"].clone()), (json!("user"), v["message_id"].clone()));
    let text = m["text"].as_str().unwrap();
    assert!(text.starts_with("2 notes on #212 pixel/inbox-empty (at aaa1111), from the review:\n"), "{text}");
    assert!(text.contains("\n1. src/InboxEmpty.tsx:40-41\n   40 |     <section aria-label=\"Inbox\">\n   41 |   <div>\n   → Use aria-labelledby.\n"), "{text}");
    assert!(text.contains("\n2. src/InboxEmpty.tsx:82\n   82 | B\n   → Why B?\n"), "{text}");
    let meta: Value = serde_json::from_str(m["meta"].as_str().unwrap()).unwrap();
    assert_eq!(meta["kind"], "pr_notes");
    assert_eq!((meta["pr"].clone(), meta["card_id"].clone(), meta["author_bot_id"].clone()), (json!(212), json!(card.id), json!(env.a)));
    assert_eq!(meta["pr_url"], "https://github.com/demo-org/app/pull/212");
    assert_eq!(meta["notes"][0]["id"], a["id"]);
    assert_eq!(meta["notes"][0]["at"], "src/InboxEmpty.tsx:40-41");
    assert_eq!(meta["notes"][1]["id"], b["id"]);

    // The same send_id: the first result, nothing posted
    let (s, again) = call(&env.app, "POST", &send, Some(json!({"send_id": "s-1"}))).await;
    assert_eq!((s.as_u16(), again["message_id"].clone()), (200, v["message_id"].clone()));
    let (s, r) = call(&env.app, "POST", &send, Some(json!({"send_id": "s-1", "ids": [a["id"]]}))).await;
    assert_eq!((s.as_u16(), r["code"].clone()), (409, json!("send_id_reused")));
    let (_, msgs) = call(&env.app, "GET", &format!("/api/channels/{}/messages", env.a), None).await;
    assert_eq!(msgs.as_array().unwrap().iter().filter(|m| m["text"].as_str().is_some_and(|t| t.contains("from the review"))).count(), 1);

    // Nothing left to send; a sent note can't change
    let (s, r) = call(&env.app, "POST", &send, Some(json!({"send_id": "s-2"}))).await;
    assert_eq!((s.as_u16(), r["code"].clone()), (422, json!("empty")));
    let (s, r) = call(&env.app, "PATCH", &format!("/api/dev/notes/{}", a["id"].as_str().unwrap()), Some(json!({"text": "y"}))).await;
    assert_eq!((s.as_u16(), r["code"].clone()), (409, json!("not_draft")));
    let (s, r) = call(&env.app, "POST", &format!("/api/dev/notes/{}/restore", a["id"].as_str().unwrap()), None).await;
    assert_eq!((s.as_u16(), r["code"].clone()), (409, json!("not_draft")));
}

#[tokio::test]
async fn a_push_resolves_the_notes_whose_lines_changed() {
    let env = setup().await;
    let card = review(&env, "aaa1111", PATCH).await;
    let notes = format!("/api/dev/cards/{}/notes", card.id);
    let new = |l: u32| json!({"side": "new", "line": l});
    let (_, a) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(40), new(41), "label"))).await;
    let (_, b) = call(&env.app, "POST", &notes, Some(note("aaa1111", new(82), new(82), "B"))).await;
    let (s, _) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "s-1"}))).await;
    assert_eq!(s.as_u16(), 200);

    // The bot pushes: a line added above the first note, the second note's line edited
    let pushed = PATCH.replace("@@ -38,6 +38,7 @@ export function InboxEmpty\n", "@@ -38,6 +38,8 @@ export function InboxEmpty\n+// moved\n").replace("+B\n", "+Bee\n");
    let old_card = env.state.db.set_dev_card(&card.id, STALE, None).await.unwrap().unwrap();
    let newer = review(&env, "bbb2222", &pushed).await;
    line_notes::after_push(env.state.clone(), newer.clone()).await;

    // Read through the old card's id too: the notes belong to the PR
    for id in [&old_card.id, &newer.id] {
        let (_, l) = call(&env.app, "GET", &format!("/api/dev/cards/{id}/notes"), None).await;
        let by = |nid: &Value| l["notes"].as_array().unwrap().iter().find(|n| n["id"] == *nid).unwrap().clone();
        let (na, nb) = (by(&a["id"]), by(&b["id"]));
        assert_eq!((na["state"].clone(), na["checked_sha"].clone()), (json!("open"), json!("bbb2222")), "{l}");
        assert_eq!(na["at_head"], json!({"start": {"side": "new", "line": 41}, "end": {"side": "new", "line": 42}}));
        assert_eq!(na["seen_sha"], "bbb2222");
        assert_eq!((nb["state"].clone(), nb["at_head"].clone()), (json!("resolved"), Value::Null));
        assert_eq!(l["summary"], json!({"total": 2, "changed": 1, "open": 1, "sha": "bbb2222"}));
    }

    // A new note now anchors to the new head only
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{}/notes", old_card.id), Some(note("aaa1111", new(40), new(40), "x"))).await;
    assert_eq!((s.as_u16(), v["code"].clone(), v["head_sha"].clone()), (409, json!("stale_head"), json!("bbb2222")));
}

#[tokio::test]
async fn a_merged_or_closed_pr_is_read_only_and_a_person_s_pr_has_no_author() {
    let env = setup().await;
    let space = SpaceId::default_space();
    // A PR by the person: no bot, a branch with no bot's name
    let row = env
        .state
        .db
        .insert_dev_card_in(space.as_str(), REPO, REVIEW, "pr:7", "ccc", &facts("ccc", None, "oleks/tweak", PATCH).replace("212", "7"))
        .await
        .unwrap()
        .unwrap();
    let notes = format!("/api/dev/cards/{}/notes", row.id);
    let (s, _) = call(&env.app, "POST", &notes, Some(note("ccc", json!({"side": "new", "line": 40}), json!({"side": "new", "line": 40}), "x"))).await;
    assert_eq!(s.as_u16(), 201);
    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "p-1"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("no_author")), "{v}");

    // Closed: its newest card went stale with no landed card
    env.state.db.set_dev_card(&row.id, STALE, None).await.unwrap();
    let (_, c) = call(&env.app, "GET", &format!("/api/dev/cards/{}/changes", row.id), None).await;
    assert_eq!(c["pr_state"], "closed");
    let (s, v) = call(&env.app, "POST", &notes, Some(note("ccc", json!({"side": "new", "line": 41}), json!({"side": "new", "line": 41}), "x"))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("pr_closed")));

    // Merged: a landed card came from it
    let landed = facts("ddd", None, "", "").replace("\"kind\":\"review\"", "\"kind\":\"landed\"").replace("212", "7");
    env.state.db.insert_dev_card_in(space.as_str(), REPO, LANDED, "ddd", "ddd", &landed).await.unwrap().unwrap();
    let (_, c) = call(&env.app, "GET", &format!("/api/dev/cards/{}/changes", row.id), None).await;
    assert_eq!(c["pr_state"], "merged");
    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "p-2"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("pr_merged")));
    // Reading still works
    let (s, l) = call(&env.app, "GET", &notes, None).await;
    assert_eq!((s.as_u16(), l["notes"].as_array().unwrap().len()), (200, 1));
}
