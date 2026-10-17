//! File attachments (batch 2 / D): uploads are capped and sniffed, stored by content hash, served
//! only behind auth with safe headers; agents read their chat's files and share files back as
//! download cards; no path from a client or an agent can reach outside the store or workspace.

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
};
use common::*;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use serde_json::{json, Value};
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR-not-a-real-image-but-png-magic";

async fn upload(env: &Env, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/attachments?name={}", urlencode(name)))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let resp = env.app.clone().oneshot(req).await.unwrap();
    let s = resp.status();
    let b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn fetch(env: &Env, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut b = Request::builder().uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = env.app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let (parts, body) = resp.into_parts();
    (parts.status, parts.headers, to_bytes(body, usize::MAX).await.unwrap().to_vec())
}

fn store_files(env: &Env) -> Vec<std::path::PathBuf> {
    let root = env.tmp.path().join("data/attachments");
    let mut out = Vec::new();
    for d in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        if d.file_name() == "tmp" {
            continue;
        }
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            out.push(f.path());
        }
    }
    out
}

fn tmp_files(env: &Env) -> usize {
    std::fs::read_dir(env.tmp.path().join("data/attachments/tmp")).map(|d| d.count()).unwrap_or(0)
}

async fn self_call(env: &Env, token: &str, tool: &str, args: Value) -> Value {
    let (s, v) = call_with(
        &env.app,
        "POST",
        "/api/mcp/self",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})),
        &[("authorization", &format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    v["result"].clone()
}

fn text_of(result: &Value) -> String {
    result["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

/// A token for a turn of `kwant` in its direct chat, optionally with a workspace.
async fn direct_turn(env: &Env, token: &str, kwant: &str, workspace: Option<std::path::PathBuf>) {
    env.state.self_tools.lock().await.insert(
        token.into(),
        SelfScope {
            kwant_id: kwant.into(),
            kwant_name: "K".into(),
            origin: Some(TurnOrigin { direct_kwant_id: Some(kwant.into()), ..Default::default() }),
            workspace,
            ..Default::default()
        },
    );
}

#[tokio::test]
async fn uploads_are_sniffed_stored_once_and_served_safely() {
    let env = setup().await;
    let (s, a) = upload(&env, "photo.txt", PNG.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    assert_eq!(a["mime"], "image/png", "the bytes decide, not the name");
    assert_eq!(a["size"], PNG.len());
    assert_eq!(a["source"], "user");
    let id = a["id"].as_str().unwrap();
    assert_eq!(a["url"], format!("/api/attachments/{id}/content"));
    assert!(!a.to_string().contains(env.tmp.path().to_str().unwrap()), "no disk paths in the API");

    let (s, h, body) = fetch(&env, &format!("/api/attachments/{id}/content"), &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, PNG);
    assert_eq!(h["content-type"], "image/png");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(h["content-security-policy"].to_str().unwrap().contains("sandbox"));
    assert!(h["content-disposition"].to_str().unwrap().starts_with("inline;"));
    let (_, h, _) = fetch(&env, &format!("/api/attachments/{id}/content?download=1"), &[]).await;
    assert!(h["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    let (s, _, body) = fetch(&env, &format!("/api/attachments/{id}/content"), &[("range", "bytes=0-3")]).await;
    assert_eq!((s, body.as_slice()), (StatusCode::PARTIAL_CONTENT, &PNG[..4]));

    // Same bytes again: a second row, one blob
    let (s, b) = upload(&env, "again.png", PNG.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(a["sha256"], b["sha256"]);
    assert_eq!(store_files(&env).len(), 1);
    let sha = a["sha256"].as_str().unwrap();
    assert!(store_files(&env)[0].ends_with(format!("{}/{sha}", &sha[..2])));

    // HTML/SVG are stored as plain text and always download
    let (s, h) = upload(&env, "page.html", b"<html><script>alert(1)</script>".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(h["mime"], "text/plain");
    let (_, hh, _) = fetch(&env, &format!("/api/attachments/{}/content", h["id"].as_str().unwrap()), &[]).await;
    assert_eq!(hh["content-type"], "text/plain");
    assert!(hh["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    let (_, md) = upload(&env, "notes.md", b"# Notes\n".to_vec()).await;
    assert_eq!(md["mime"], "text/markdown");
}

#[tokio::test]
async fn bad_uploads_are_refused_and_leave_nothing_behind() {
    let env = setup().await;
    let (s, _) = upload(&env, "setup.png", b"MZ\x90\0\x03\0\0\0binary".to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (s, _) = upload(&env, "x.bin", vec![0xff, 0xfe, 0x00, 0x01]).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (s, _) = upload(&env, "empty.txt", Vec::new()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = upload(&env, "big.txt", vec![b'a'; 20 * 1024 * 1024 + 1]).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(tmp_files(&env), 0, "rejected uploads are deleted");
    assert!(store_files(&env).is_empty());
}

#[tokio::test]
async fn names_and_ids_cannot_traverse() {
    let env = setup().await;
    let (s, a) = upload(&env, "../../../etc/passwd", b"root:x:0:0".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(a["name"], "passwd");
    let (_, a) = upload(&env, "..\\..\\boot.ini", b"x".to_vec()).await;
    assert_eq!(a["name"], "boot.ini");
    let (_, a) = upload(&env, "..", b"x".to_vec()).await;
    assert_eq!(a["name"], "file");
    assert!(!env.tmp.path().join("etc").exists());
    for f in store_files(&env) {
        assert!(f.starts_with(env.tmp.path().join("data/attachments")));
    }

    for uri in [
        "/api/attachments/..%2F..%2Fconfig.json/content",
        "/api/attachments/../../config.json/content",
        "/api/attachments/%2e%2e/content",
        "/api/attachments/not-a-uuid/content",
        "/api/attachments/00000000-0000-0000-0000-000000000000/content",
        "/api/attachments/00000000-0000-0000-0000-000000000000",
    ] {
        let (s, h, body) = fetch(&env, uri, &[]).await;
        // Unmatched raw `../` paths fall through to the SPA shell (HTML), never to a file
        let shell = s == StatusCode::OK && h["content-type"].to_str().unwrap().starts_with("text/html");
        assert!(shell || s == StatusCode::NOT_FOUND || s == StatusCode::BAD_REQUEST, "{uri}: {s}");
        assert!(!String::from_utf8_lossy(&body).contains("root:x"));
    }
    let (s, _) = call(&env.app, "GET", "/api/attachments", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "context_id is required");
}

#[tokio::test]
async fn sent_files_reach_the_agent_and_only_its_chat() {
    let env = setup().await;
    let (_, md) = upload(&env, "plan.md", b"STEP_ONE_SECRET_PLAN\nstep two".to_vec()).await;
    let (_, png) = upload(&env, "shot.png", PNG.to_vec()).await;
    let (_, loose) = upload(&env, "loose.txt", b"never sent".to_vec()).await;
    let ids = [md["id"].as_str().unwrap(), png["id"].as_str().unwrap()];

    send_direct(&env, &env.a, "look at these REPLYTAG:f1", json!({"attachment_ids": ids})).await;
    wait_reply(&env, &env.a, "answer f1").await;
    let args = invocations_with("REPLYTAG:f1").pop().unwrap();
    assert!(args.contains("[Attached files]") && args.contains("shot.png (image/png"));
    assert!(args.contains("STEP_ONE_SECRET_PLAN"), "small text files are inlined");
    let user = messages(&env, &env.a).await.into_iter().find(|m| m.text == "look at these REPLYTAG:f1").unwrap();
    let meta: Value = serde_json::from_str(user.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(meta["attachments"][0]["name"], "plan.md");
    assert!(!user.text.contains("[Attached files]"), "the chat shows what the user typed");

    let (_, list) = call(&env.app, "GET", &format!("/api/attachments?context_id={}", env.a), None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);

    // Sending again, unknown ids, too many: refused
    let body = |ids: Value| json!({"prompt": "x", "sender_id": "user", "kwant_id": env.a, "attachment_ids": ids});
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(body(json!([ids[0]])))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(body(json!(["../../etc/passwd"])))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(body(json!(vec!["x"; 11])))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The agent reads its chat's files: text as text, image as an image
    direct_turn(&env, "tok-a", &env.a, None).await;
    let r = self_call(&env, "tok-a", "read_attachment", json!({"id": ids[0]})).await;
    assert_eq!(r["isError"], false);
    assert!(text_of(&r).contains("STEP_ONE_SECRET_PLAN"));
    let r = self_call(&env, "tok-a", "read_attachment", json!({"id": ids[1]})).await;
    assert_eq!(r["content"][1]["type"], "image");
    assert_eq!(r["content"][1]["mimeType"], "image/png");
    let r = self_call(&env, "tok-a", "list_attachments", json!({})).await;
    assert!(text_of(&r).contains("plan.md") && text_of(&r).contains(ids[1]));

    // Another agent's chat, an unsent upload, or a made-up id: not visible
    direct_turn(&env, "tok-b", &env.b, None).await;
    let r = self_call(&env, "tok-b", "read_attachment", json!({"id": ids[0]})).await;
    assert_eq!(r["isError"], true);
    assert!(!text_of(&r).contains("STEP_ONE"));
    for id in [loose["id"].as_str().unwrap(), "../../config.json", ""] {
        let r = self_call(&env, "tok-a", "read_attachment", json!({"id": id})).await;
        assert_eq!(r["isError"], true, "{id}");
    }

    // Through a real turn of the fake CLI as well
    let marker = call_tool_marker("read_attachment", json!({"id": ids[0]}));
    send_direct(&env, &env.a, &format!("{marker} REPLYTAG:f2"), json!({})).await;
    let reply = wait_reply(&env, &env.a, "answer f2").await;
    assert!(reply.text.contains("STEP_ONE_SECRET_PLAN"), "{}", reply.text);
}

#[tokio::test]
async fn agents_share_files_as_download_cards_from_inside_their_workspace_only() {
    let env = setup().await;
    let ws = env.tmp.path().join("team-ws");
    std::fs::create_dir_all(ws.join("out")).unwrap();
    std::fs::write(ws.join("out/report.csv"), "a,b\n1,2\n").unwrap();
    std::fs::write(env.tmp.path().join("outside.txt"), "OUTSIDE_SECRET").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(env.tmp.path().join("outside.txt"), ws.join("sneaky.txt")).unwrap();
    direct_turn(&env, "tok-ws", &env.a, Some(ws.clone())).await;
    let mut rx = env.state.event_sender.subscribe();

    let r = self_call(&env, "tok-ws", "share_file", json!({"path": "out/report.csv", "name": "report.csv"})).await;
    assert_eq!(r["isError"], false, "{r}");
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text == "Shared a file: report.csv").unwrap();
    assert_eq!(card.sender_role, "kwant");
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["kind"], "file");
    assert_eq!(meta["attachments"][0]["mime"], "text/csv");
    let aid = meta["attachments"][0]["id"].as_str().unwrap();
    let (s, h, body) = fetch(&env, &format!("/api/attachments/{aid}/content"), &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, b"a,b\n1,2\n");
    assert!(h["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    let mut announced = false;
    while let Ok(e) = rx.try_recv() {
        if let kwnt::RuntimeEvent::MessageAdded { message } = e.unscoped() {
            announced |= message["id"] == card.id.as_str();
        }
    }
    assert!(announced, "message_added for the card");
    let (_, a) = call(&env.app, "GET", &format!("/api/attachments/{aid}"), None).await;
    assert_eq!((a["source"].as_str(), a["kwant_id"].as_str()), (Some("agent"), Some(env.a.as_str())));

    for bad in [
        json!({"path": "../outside.txt", "name": "x"}),
        json!({"path": env.tmp.path().join("outside.txt").to_str().unwrap(), "name": "x"}),
        json!({"path": "sneaky.txt", "name": "x"}),
        json!({"path": "out", "name": "x"}),
        json!({"path": "/etc/passwd", "name": "x"}),
        json!({"name": "x"}),
        json!({"path": "out/report.csv", "content": "both", "name": "x"}),
    ] {
        let r = self_call(&env, "tok-ws", "share_file", bad.clone()).await;
        assert_eq!(r["isError"], true, "{bad}");
    }
    let (_, list) = call(&env.app, "GET", &format!("/api/attachments?context_id={}", env.a), None).await;
    assert!(!list.to_string().contains("OUTSIDE"));
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Without a workspace only written content can be shared
    direct_turn(&env, "tok-nows", &env.b, None).await;
    let r = self_call(&env, "tok-nows", "share_file", json!({"path": "out/report.csv", "name": "r.csv"})).await;
    assert_eq!(r["isError"], true);
    let r = self_call(&env, "tok-nows", "share_file", json!({"content": "# Summary\nall good", "name": "../../summary.md"})).await;
    assert_eq!(r["isError"], false);
    let card = messages(&env, &env.b).await.into_iter().find(|m| m.text.starts_with("Shared a file")).unwrap();
    assert_eq!(card.text, "Shared a file: summary.md");
    // The agent that shared it can read it back; it's visible in that chat
    let id = serde_json::from_str::<Value>(card.meta.as_deref().unwrap()).unwrap()["attachments"][0]["id"].as_str().unwrap().to_string();
    let r = self_call(&env, "tok-nows", "read_attachment", json!({"id": id})).await;
    assert!(text_of(&r).contains("all good"));
    let r = self_call(&env, "tok-ws", "read_attachment", json!({"id": id})).await;
    assert_eq!(r["isError"], true, "agent a can't read b's file");
}

#[tokio::test]
async fn every_attachment_route_needs_auth() {
    let env = setup().await;
    let (_, a) = upload(&env, "x.txt", b"private".to_vec()).await;
    let id = a["id"].as_str().unwrap();
    for (m, u) in [
        ("GET", "/api/attachments?context_id=x".to_string()),
        ("POST", "/api/attachments?name=x.txt".to_string()),
        ("GET", format!("/api/attachments/{id}")),
        ("GET", format!("/api/attachments/{id}/content")),
    ] {
        assert_eq!(call_remote(&env.app, m, &u).await, StatusCode::UNAUTHORIZED, "{m} {u}");
    }
    let (s, _, body) = fetch(&env, &format!("/api/attachments/{id}/content"), &[("host", "kwnt.example.com")]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(!String::from_utf8_lossy(&body).contains("private"));
}

#[tokio::test]
async fn attachments_migration_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let db = kwnt::db::Database::init(tmp.path()).await.unwrap();
    db.insert_attachment(&kwnt::db::attachments::NewAttachment {
        sha256: "a".repeat(64),
        name: "x".into(),
        mime: "text/plain".into(),
        size: 1,
        source: "user".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    drop(db);
    let db = kwnt::db::Database::init(tmp.path()).await.unwrap();
    assert!(db.get_attachment("nope").await.unwrap().is_none());
}
