//! Rigel QA probe (not for merge): the global upload slots, in their own test binary because the slots are process-wide.
//! the upload slots are process-wide, so this is its own test binary.
#![allow(dead_code, unused)]

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    pocket::{self, guest, listener},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";
const REMOTE: &str = "203.0.113.9:50000";
const GHOST: &str = "pocket.example.net";
const PIN: &str = "4821";

struct T {
    main: Router,
    guest: Router,
    state: AppState,
    data: PathBuf,
    _tmp: TempDir,
}

async fn setup() -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        pocket_port: Some(0),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    listener::reconcile(&state).await;
    T {
        main: create_router(state.clone()),
        guest: guest::router(state.clone()),
        state,
        data,
        _tmp: tmp,
    }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
    body: Value,
}

impl Res {
    fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|c| {
                let first = c.split(';').next()?;
                let (k, v) = first.split_once('=')?;
                (k == name).then(|| v.to_string())
            })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

#[derive(Default, Clone)]
struct Req {
    cookies: Vec<(String, String)>,
    bearer: Option<String>,
    ip: Option<String>,
    peer: Option<&'static str>,
    headers: Vec<(&'static str, String)>,
}

enum B {
    None,
    Json(Value),
    Raw(Vec<u8>),
}

async fn send(app: &Router, method: &str, uri: &str, body: B, r: &Req) -> Res {
    let host = match r.peer {
        Some(LOCAL) => "127.0.0.1:8799",
        Some(_) => "kwnt.example.com",
        None => GHOST,
    };
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host);
    if !r.cookies.is_empty() {
        req = req.header(
            "cookie",
            r.cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    if let Some(b) = &r.bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    if let Some(ip) = &r.ip {
        req = req.header("x-forwarded-for", ip.as_str());
    }
    for (k, v) in &r.headers {
        req = req.header(*k, v.as_str());
    }
    let body = match body {
        B::None => Body::empty(),
        B::Json(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        B::Raw(b) => Body::from(b),
    };
    let mut req = req.body(body).unwrap();
    if let Some(p) = r.peer {
        req.extensions_mut()
            .insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Res {
        status,
        headers,
        bytes,
        body,
    }
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

fn person() -> Req {
    Req {
        peer: Some(LOCAL),
        ..Default::default()
    }
}

fn guest_req(code: &str, key: &str) -> Req {
    Req {
        cookies: vec![(guest::guest_cookie(code), key.to_string())],
        ..Default::default()
    }
}

fn host_req(key: &str) -> Req {
    Req {
        bearer: Some(key.to_string()),
        ..Default::default()
    }
}

/// Makes a wall through the person's API: (code, host key).
async fn make(t: &T, spec: Value) -> (String, String) {
    let r = send(
        &t.main,
        "POST",
        "/api/pockets",
        B::Json(json!({ "kit": "wall", "spec": spec })),
        &person(),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (
        r.body["pocket"]["code"].as_str().unwrap().to_string(),
        r.body["host_key"].as_str().unwrap().to_string(),
    )
}

async fn join(t: &T, code: &str, name: &str) -> Res {
    send(
        &t.guest,
        "POST",
        &format!("/api/pocket/{code}/join"),
        B::Json(json!({ "name": name, "pin": PIN })),
        &Req {
            ip: Some(fresh_ip()),
            ..Default::default()
        },
    )
    .await
}

/// Joins and returns (player id, guest key).
async fn joined(t: &T, code: &str, name: &str) -> (String, String) {
    let r = join(t, code, name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{name}: {}", r.body);
    (
        r.body["me"]["id"].as_str().unwrap().to_string(),
        r.cookie(&guest::guest_cookie(code)).unwrap(),
    )
}

/// One photo from a guest, as the page sends it: the raw file, `Content-Length` set unless
/// `chunked`, the caption URI-encoded in `X-Caption`.
async fn upload_as(
    t: &T,
    code: &str,
    key: &str,
    file: &[u8],
    caption: Option<&str>,
    chunked: bool,
) -> Res {
    let mut r = guest_req(code, key);
    r.headers.push(("content-type", "image/jpeg".into()));
    if !chunked {
        r.headers.push(("content-length", file.len().to_string()));
    }
    if let Some(c) = caption {
        r.headers.push(("x-caption", percent_encode(c)));
    }
    send(
        &t.guest,
        "POST",
        &format!("/api/pocket/{code}/photo"),
        B::Raw(file.to_vec()),
        &r,
    )
    .await
}

async fn upload(t: &T, code: &str, key: &str, file: &[u8]) -> Res {
    upload_as(t, code, key, file, None, false).await
}

/// Uploads and returns the new photo's id.
async fn sent(t: &T, code: &str, key: &str, file: &[u8]) -> String {
    let r = upload(t, code, key, file).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["photo"]["id"].as_str().unwrap().to_string()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn host_act(t: &T, code: &str, name: &str, body: Value) -> Res {
    send(
        &t.main,
        "POST",
        &format!("/api/pockets/{code}/action"),
        B::Json(json!({ "name": name, "body": body })),
        &person(),
    )
    .await
}

async fn guest_act(t: &T, code: &str, key: &str, name: &str, body: Value) -> Res {
    tokio::time::sleep(std::time::Duration::from_millis(110)).await;
    send(
        &t.guest,
        "POST",
        &format!("/api/pocket/{code}/action"),
        B::Json(json!({ "name": name, "body": body })),
        &guest_req(code, key),
    )
    .await
}

async fn view_of(t: &T, code: &str, key: &str) -> Value {
    let r = send(
        &t.guest,
        "GET",
        &format!("/api/pocket/{code}/state"),
        B::None,
        &guest_req(code, key),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

async fn get_photo(t: &T, code: &str, id: &str, r: &Req) -> Res {
    send(
        &t.guest,
        "GET",
        &format!("/api/pocket/{code}/photo/{id}"),
        B::None,
        r,
    )
    .await
}

/// A screen link signed in: the screen's cookie as a request.
async fn screen(t: &T, code: &str) -> Req {
    let r = send(
        &t.main,
        "POST",
        &format!("/api/pockets/{code}/link"),
        B::Json(json!({ "role": "screen" })),
        &person(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let tok = r.body["token"].as_str().unwrap().to_string();
    let r = send(
        &t.guest,
        "POST",
        &format!("/api/pocket/{code}/host/signin"),
        B::Json(json!({ "token": tok })),
        &Req {
            ip: Some(fresh_ip()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    Req {
        cookies: vec![(
            guest::host_cookie(code),
            r.cookie(&guest::host_cookie(code)).unwrap(),
        )],
        ..Default::default()
    }
}

fn wall_dir(t: &T, code: &str) -> PathBuf {
    t.data.join("pocket").join(code).join("wall")
}

fn files(t: &T, code: &str) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = match std::fs::read_dir(wall_dir(t, code)) {
        Ok(d) => d
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Nothing that names a place, a camera or a person: every canary below, and the metadata
/// blocks themselves.
fn assert_clean(what: &str, b: &[u8]) {
    for bad in [
        &b"CANARY"[..],
        b"Exif\0",
        b"xmpmeta",
        b"Photoshop 3.0",
        b"XMP ",
        b"EXIF",
    ] {
        assert!(
            !has(b, bad),
            "{what} still has {:?}",
            String::from_utf8_lossy(bad)
        );
    }
}

// ---- Photo files with metadata ----

/// A little-endian TIFF block (Exif's body): the orientation, and a Make string standing in for
/// the camera and GPS tags.
fn tiff(orientation: u16) -> Vec<u8> {
    let make = b"GPS-CANARY 46.0569N 14.5058E iPhone\0";
    let mut t = b"II*\0".to_vec();
    t.extend(8u32.to_le_bytes());
    t.extend(2u16.to_le_bytes());
    t.extend(0x0112u16.to_le_bytes());
    t.extend(3u16.to_le_bytes());
    t.extend(1u32.to_le_bytes());
    t.extend(orientation.to_le_bytes());
    t.extend([0, 0]);
    t.extend(0x010Fu16.to_le_bytes());
    t.extend(2u16.to_le_bytes());
    t.extend((make.len() as u32).to_le_bytes());
    t.extend((8u32 + 2 + 24 + 4).to_le_bytes());
    t.extend(0u32.to_le_bytes());
    t.extend(make);
    t
}

fn seg(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut s = vec![0xFF, marker];
    s.extend(((payload.len() + 2) as u16).to_be_bytes());
    s.extend(payload);
    s
}

/// A JPEG 640x480 with Exif (orientation 6, so 90°), XMP, IPTC, a comment, `pad` bytes of scan
/// data and a trailer after the end marker, each carrying a canary.
fn jpeg_padded(pad: usize) -> Vec<u8> {
    let mut j = vec![0xFF, 0xD8];
    j.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(tiff(6));
    j.extend(seg(0xE1, &exif));
    j.extend(seg(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta><exif:GPSLatitude>XMP-CANARY</exif:GPSLatitude></x:xmpmeta>"));
    j.extend(seg(0xED, b"Photoshop 3.0\0IPTC-CANARY by Mia"));
    j.extend(seg(0xFE, b"COMMENT-CANARY"));
    j.extend(seg(0xC0, &[8, 0x01, 0xE0, 0x02, 0x80, 1, 1, 0x11, 0]));
    j.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
    j.extend([0x12, 0x34, 0xFF, 0x00, 0x56]);
    j.extend(std::iter::repeat_n(0x11, pad));
    j.extend([0xFF, 0xD9]);
    j.extend(b"TRAILER-CANARY");
    j
}

fn jpeg() -> Vec<u8> {
    jpeg_padded(64)
}

fn crc32(b: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in b {
        c ^= u32::from(x);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    c.extend(ty);
    c.extend(data);
    let mut crc_in = ty.to_vec();
    crc_in.extend(data);
    c.extend(crc32(&crc_in).to_be_bytes());
    c
}

/// A PNG 3x2 with eXIf (orientation 3, so 180°) and text chunks carrying canaries.
fn png() -> Vec<u8> {
    let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
    p.extend(chunk(b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0]));
    p.extend(chunk(b"eXIf", &tiff(3)));
    p.extend(chunk(b"tEXt", b"Comment\0TEXT-CANARY"));
    p.extend(chunk(
        b"iTXt",
        b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta>ITXT-CANARY</x:xmpmeta>",
    ));
    p.extend(chunk(b"zTXt", b"Author\0\0ZTXT-CANARY"));
    p.extend(chunk(b"tIME", &[0x07, 0xEA, 11, 24, 21, 14, 0]));
    p.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01],
    ));
    p.extend(chunk(b"IEND", &[]));
    p
}

fn riff_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = ty.to_vec();
    c.extend((data.len() as u32).to_le_bytes());
    c.extend(data);
    if data.len() % 2 == 1 {
        c.push(0);
    }
    c
}

/// A WebP 800x600 (VP8X, flags say EXIF and XMP) with EXIF (orientation 8, so 270°) and XMP.
fn webp() -> Vec<u8> {
    let mut vp8x = vec![0x08 | 0x04, 0, 0, 0];
    vp8x.extend(&799u32.to_le_bytes()[..3]);
    vp8x.extend(&599u32.to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &vp8x));
    body.extend(riff_chunk(
        b"VP8L",
        &[0x2F, 0x1F, 0xC0, 0x95, 0x00, 0x07, 0x10, 0x11],
    ));
    body.extend(riff_chunk(b"EXIF", &tiff(8)));
    body.extend(riff_chunk(
        b"XMP ",
        b"<x:xmpmeta>WEBP-XMP-CANARY</x:xmpmeta>",
    ));
    let mut w = b"RIFF".to_vec();
    w.extend((body.len() as u32).to_le_bytes());
    w.extend(body);
    w
}

fn unzip(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(b.to_vec())).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut out = Vec::new();
            f.read_to_end(&mut out).unwrap();
            (f.name().unwrap().to_string(), out)
        })
        .collect()
}

fn spec(extra: Value) -> Value {
    let mut s = json!({ "title": "Mia & Sam's wedding", "host_name": "Oleks" });
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    s
}

/// A photo upload whose body sends the first bytes and then stalls (a slow phone on bad Wi-Fi,
/// or a hostile guest). Returns the sender that keeps it open, and the response future.
fn stalled_upload(
    t: &T,
    code: &str,
    key: &str,
    file: &[u8],
) -> (
    tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    tokio::task::JoinHandle<StatusCode>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(2);
    let first = Bytes::from(file[..16].to_vec());
    let tx2 = tx.clone();
    tokio::spawn(async move {
        let _ = tx2.send(Ok(first)).await;
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/pocket/{code}/photo"))
        .header("host", GHOST)
        .header("cookie", format!("{}={key}", guest::guest_cookie(code)))
        .header("content-type", "image/jpeg")
        .header("content-length", file.len().to_string())
        .body(body)
        .unwrap();
    let app = t.guest.clone();
    let h = tokio::spawn(async move { app.oneshot(req).await.unwrap().status() });
    (tx, h)
}

/// F2: 4 stalled uploads take all the slots; the 5th waits 20 s and gets 429 + Retry-After 3
/// without a byte stored; when the stalled clients go away the slots come back.
/// Reports whether a stalled client can hold a slot for as long as it likes (no body timeout).
#[tokio::test]
async fn qa_upload_slots_busy_wait_then_429_and_release_on_disconnect() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({ "per_guest_max": 50 }))).await;
    let mut keys = Vec::new();
    for i in 0..6 {
        keys.push(joined(&t, &code, &format!("S{i}")).await.1);
    }
    let file = jpeg();
    let mut held = Vec::new();
    for k in keys.iter().take(4) {
        held.push(stalled_upload(&t, &code, k, &file));
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let t0 = std::time::Instant::now();
    let r = upload(&t, &code, &keys[4], &file).await;
    let waited = t0.elapsed().as_secs_f32();
    eprintln!(
        "QA-REPORT slots: 5th upload -> {} {:?} retry-after={:?} after {waited:.1}s",
        r.status,
        r.body["error"],
        r.header("retry-after")
    );
    assert_eq!(
        (r.status, r.body["error"].as_str(), r.header("retry-after")),
        (StatusCode::TOO_MANY_REQUESTS, Some("slow_down"), Some("3"))
    );
    assert!((19.0..25.0).contains(&waited), "waits about 20 s: {waited}");
    assert!(
        files(&t, &code).is_empty(),
        "nothing stored for the refused or the stalled uploads"
    );
    // are the stalled ones still holding their slot after 20+ s? (no body timeout in the server)
    let still = held.iter().filter(|(_, h)| !h.is_finished()).count();
    eprintln!("QA-REPORT slots: stalled uploads still open after {:.0}s: {still} of 4 (a client can hold a slot as long as it keeps the connection)", t0.elapsed().as_secs_f32());
    // the clients go away: slots free up
    for (tx, h) in held {
        drop(tx);
        h.abort();
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let r = upload(&t, &code, &keys[5], &file).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "slots are released when the clients go away: {}",
        r.body
    );
}
