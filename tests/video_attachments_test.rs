//! Videos as attachments (video-drop-073): mp4/mov/webm are recognised from their bytes, may be
//! up to 100 MB (everything else stays at 20 MB), are refused right after the first bytes when
//! they can't be accepted, play inline, are shared by agents, and read as a few frames.
//! Fixtures are 2–3 s ffmpeg `testsrc` clips (synthetic, no real footage).

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
};
use common::*;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

const MP4: &[u8] = include_bytes!("fixtures/testsrc.mp4");
const MOV: &[u8] = include_bytes!("fixtures/testsrc.mov");
const WEBM: &[u8] = include_bytes!("fixtures/testsrc.webm");

async fn upload_body(env: &Env, name: &str, body: Body, length: Option<u64>) -> (StatusCode, Value) {
    let mut req = Request::builder().method("POST").uri(format!("/api/attachments?name={name}"));
    if let Some(n) = length {
        req = req.header("content-length", n.to_string());
    }
    let resp = env.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn upload(env: &Env, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    upload_body(env, name, Body::from(bytes), None).await
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

fn tmp_files(env: &Env) -> usize {
    std::fs::read_dir(env.tmp.path().join("data/attachments/tmp")).map(|d| d.count()).unwrap_or(0)
}

/// A video head (`ftyp isom`) padded with zeros to `len` bytes.
fn fake_mp4(len: usize) -> Vec<u8> {
    let mut v = b"\0\0\0\x20ftypisom\0\0\x02\0isomiso2avc1mp41".to_vec();
    v.resize(len, 0);
    v
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
async fn videos_are_known_by_their_bytes_and_play_inline() {
    let env = setup().await;
    // Misleading names on purpose: the bytes decide
    for (name, bytes, mime) in [("clip.mov", MP4, "video/mp4"), ("clip.mp4", MOV, "video/quicktime"), ("clip.txt", WEBM, "video/webm")] {
        let (s, a) = upload(&env, name, bytes.to_vec()).await;
        assert_eq!(s, StatusCode::CREATED, "{name}: {a}");
        assert_eq!(a["mime"], mime, "{name}");
        assert_eq!(a["size"], bytes.len());
        let id = a["id"].as_str().unwrap();

        let (s, h, body) = fetch(&env, &format!("/api/attachments/{id}/content"), &[]).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, bytes);
        assert_eq!(h["content-type"], mime);
        assert!(h["content-disposition"].to_str().unwrap().starts_with("inline;"), "plays in a <video>");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["content-security-policy"], "default-src 'none'; sandbox");
        assert_eq!(h["cross-origin-resource-policy"], "same-origin");
        let (_, h, _) = fetch(&env, &format!("/api/attachments/{id}/content?download=1"), &[]).await;
        assert!(h["content-disposition"].to_str().unwrap().starts_with("attachment;"));
        // Seeking needs ranges
        let (s, h, body) = fetch(&env, &format!("/api/attachments/{id}/content"), &[("range", "bytes=100-199")]).await;
        assert_eq!(s, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, &bytes[100..200]);
        assert_eq!(h["content-range"], format!("bytes 100-199/{}", bytes.len()));
    }

    // Not videos, whatever the name: a zip, an iPhone photo, an M4A voice memo
    let mut zip = b"PK\x03\x04\x14\0\0\0\x08\0".to_vec();
    zip.resize(4096, 7);
    let mut heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
    heic.resize(4096, 0);
    let mut m4a = b"\0\0\0\x1cftypM4A \0\0\0\0M4A isom".to_vec();
    m4a.resize(4096, 0);
    for (name, bytes) in [("holiday.mp4", zip), ("IMG_0001.mov", heic), ("memo.mp4", m4a)] {
        let (s, v) = upload(&env, name, bytes).await;
        assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{name}");
        assert_eq!(v["code"], "unsupported_type");
        assert!(v["error"].as_str().unwrap().contains("videos (MP4, MOV, WebM)"));
    }
    assert_eq!(tmp_files(&env), 0);
}

#[tokio::test]
async fn videos_may_be_100_mb_and_everything_else_20() {
    let env = setup().await;
    let (s, a) = upload(&env, "long.mp4", fake_mp4(25 * 1024 * 1024)).await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    assert_eq!(a["mime"], "video/mp4");

    let (s, v) = upload(&env, "big.txt", vec![b'a'; 25 * 1024 * 1024]).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(v, json!({"error": "Files can be at most 20 MB (videos up to 100 MB).", "code": "too_large", "max_bytes": 20971520}));

    let (s, v) = upload(&env, "huge.mp4", fake_mp4(100 * 1024 * 1024 + 1)).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(v, json!({"error": "Videos can be at most 100 MB.", "code": "too_large", "max_bytes": 104857600}));
    let (s, a) = upload(&env, "edge.mp4", fake_mp4(100 * 1024 * 1024)).await;
    assert_eq!(s, StatusCode::CREATED, "exactly 100 MB is fine: {a}");

    // A declared length over 100 MB is refused before any byte is read
    let (s, v) = upload_body(&env, "x.mp4", Body::from(fake_mp4(64)), Some(100 * 1024 * 1024 + 1)).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(v["max_bytes"], 104857600);
    assert_eq!(v["error"], "Files can be at most 100 MB for videos and 20 MB for everything else.");
    assert_eq!(tmp_files(&env), 0, "refused uploads leave no temp file");
}

/// The competitor's complaint, and kwant's old behaviour: a file that can't be accepted was read
/// to the end before being refused. Now the refusal comes right after the first chunk, while
/// the client is still sending.
#[tokio::test]
async fn a_file_that_cannot_be_accepted_is_refused_before_the_body_ends() {
    let env = setup().await;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(4);
    let mut zip = b"PK\x03\x04\x14\0\0\0\x08\0".to_vec();
    zip.resize(64 * 1024, 7);
    tx.send(Ok(zip)).await.unwrap();
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let (s, v) = tokio::time::timeout(Duration::from_secs(5), upload_body(&env, "holiday.mp4", body, Some(80 * 1024 * 1024)))
        .await
        .expect("answered while the body is still open");
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{v}");
    assert_eq!(v["code"], "unsupported_type");
    // The rest is read and dropped (so a browser sees the 415, not a reset), never stored
    for _ in 0..8 {
        tx.send(Ok(vec![7u8; 1024 * 1024])).await.expect("the server keeps draining");
    }
    drop(tx);
    assert_eq!(tmp_files(&env), 0);

    // A text file with an unusual extension still goes through
    let (s, a) = upload(&env, "App.vue", b"<template><div>hello</div></template>\n".to_vec()).await;
    assert_eq!((s, a["mime"].as_str()), (StatusCode::CREATED, Some("text/plain")));
    let (s, _) = upload(&env, "server.key", b"-----BEGIN PRIVATE KEY-----\nMIIB\n".to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn agents_share_videos_and_see_them_as_frames() {
    let env = setup().await;
    let ws = env.tmp.path().join("team-ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("demo.mp4"), MP4).unwrap();
    std::fs::write(ws.join("big.mp4"), fake_mp4(100 * 1024 * 1024 + 1)).unwrap();
    std::fs::write(ws.join("big.bin.txt"), vec![b'a'; 20 * 1024 * 1024 + 1]).unwrap();
    direct_turn(&env, "tok-ws", &env.a, Some(ws.clone())).await;

    let r = self_call(&env, "tok-ws", "share_file", json!({"path": "demo.mp4", "name": "demo.mp4"})).await;
    assert_eq!(r["isError"], false, "{r}");
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text == "Shared a file: demo.mp4").unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["attachments"][0]["mime"], "video/mp4");
    let id = meta["attachments"][0]["id"].as_str().unwrap().to_string();
    let (_, h, _) = fetch(&env, &format!("/api/attachments/{id}/content"), &[]).await;
    assert!(h["content-disposition"].to_str().unwrap().starts_with("inline;"));

    let r = self_call(&env, "tok-ws", "share_file", json!({"path": "big.mp4", "name": "big.mp4"})).await;
    assert_eq!((r["isError"].as_bool(), text_of(&r).as_str()), (Some(true), "The video is larger than 100 MB."));
    let r = self_call(&env, "tok-ws", "share_file", json!({"path": "big.bin.txt", "name": "big.txt"})).await;
    assert_eq!((r["isError"].as_bool(), text_of(&r).as_str()), (Some(true), "The file is larger than 20 MB (videos up to 100 MB)."));

    let r = self_call(&env, "tok-ws", "read_attachment", json!({"id": id})).await;
    if kwnt::server::video::tools().is_none() {
        eprintln!("no ffmpeg here: checking the no-ffmpeg answer only");
        assert_eq!(r["isError"], false);
        assert!(text_of(&r).ends_with(kwnt::server::video::NO_FFMPEG), "{r}");
        return;
    }
    assert_eq!(r["isError"], false, "{r}");
    let blocks = r["content"].as_array().unwrap();
    let head = blocks[0]["text"].as_str().unwrap();
    assert!(head.starts_with("--- demo.mp4 (video/mp4, 23 KB) ---\nDuration 0:03.0 · 320×240 · h264 · 15 fps · audio aac"), "{head}");
    assert!(head.ends_with("3 frames, evenly spaced:"), "{head}");
    let images: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "image").collect();
    assert_eq!(images.len(), 3);
    assert_eq!(blocks[1]["text"], "frame 1 at 0:00.5");
    use base64::Engine;
    for img in images {
        assert_eq!(img["mimeType"], "image/jpeg");
        let jpg = base64::engine::general_purpose::STANDARD.decode(img["data"].as_str().unwrap()).unwrap();
        assert!(jpg.starts_with(&[0xFF, 0xD8, 0xFF]) && jpg.len() < 5 * 1024 * 1024);
    }
    let left: Vec<_> = std::fs::read_dir(env.tmp.path().join("data/attachments/tmp")).unwrap().flatten().collect();
    assert!(left.is_empty(), "frame temp dirs are removed: {left:?}");
}

/// A cut-off file that still starts like an mp4 is stored (the bytes say video), but ffmpeg
/// can't read it: the bot is told so instead of getting nothing.
#[tokio::test]
async fn a_damaged_video_reads_as_a_clear_error() {
    if kwnt::server::video::tools().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let env = setup().await;
    let ws = env.tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("cut.mp4"), fake_mp4(4096)).unwrap();
    direct_turn(&env, "tok", &env.a, Some(ws)).await;
    let r = self_call(&env, "tok", "share_file", json!({"path": "cut.mp4", "name": "cut.mp4"})).await;
    assert_eq!(r["isError"], false, "{r}");
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text == "Shared a file: cut.mp4").unwrap();
    let id = serde_json::from_str::<Value>(card.meta.as_deref().unwrap()).unwrap()["attachments"][0]["id"].as_str().unwrap().to_string();
    let r = self_call(&env, "tok", "read_attachment", json!({"id": id})).await;
    assert_eq!(r["isError"], true);
    assert_eq!(text_of(&r), "Couldn't look inside cut.mp4 (video/mp4, 4 KB): ffmpeg can't read it (it may be cut off or damaged).");
}
