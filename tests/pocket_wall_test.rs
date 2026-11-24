//! pa14 Wall server: location and camera details never reach the disk, the wall or the zip; who
//! sees which photo; the limits (photo, guest, wall, disk, rate, type) refuse before anything is
//! stored; captions; the zip; retention. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa14/server-brief.md, API in
//! server-impl.md next to it. The spec, views and actions alone are unit tests in
//! `src/pocket/wall.rs`; the metadata walk alone in `src/pocket/photo.rs`.

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
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, data, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
    body: Value,
}

impl Res {
    fn cookie(&self, name: &str) -> Option<String> {
        self.headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| {
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
    let mut req = Request::builder().method(method).uri(uri).header("host", host);
    if !r.cookies.is_empty() {
        req = req.header("cookie", r.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "));
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
        req.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Res { status, headers, bytes, body }
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

fn person() -> Req {
    Req { peer: Some(LOCAL), ..Default::default() }
}

fn guest_req(code: &str, key: &str) -> Req {
    Req { cookies: vec![(guest::guest_cookie(code), key.to_string())], ..Default::default() }
}

fn host_req(key: &str) -> Req {
    Req { bearer: Some(key.to_string()), ..Default::default() }
}

/// Makes a wall through the person's API: (code, host key).
async fn make(t: &T, spec: Value) -> (String, String) {
    let r = send(&t.main, "POST", "/api/pockets", B::Json(json!({ "kit": "wall", "spec": spec })), &person()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["pocket"]["code"].as_str().unwrap().to_string(), r.body["host_key"].as_str().unwrap().to_string())
}

async fn join(t: &T, code: &str, name: &str) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), B::Json(json!({ "name": name, "pin": PIN })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await
}

/// Joins and returns (player id, guest key).
async fn joined(t: &T, code: &str, name: &str) -> (String, String) {
    let r = join(t, code, name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{name}: {}", r.body);
    (r.body["me"]["id"].as_str().unwrap().to_string(), r.cookie(&guest::guest_cookie(code)).unwrap())
}

/// One photo from a guest, as the page sends it: the raw file, `Content-Length` set unless
/// `chunked`, the caption URI-encoded in `X-Caption`.
async fn upload_as(t: &T, code: &str, key: &str, file: &[u8], caption: Option<&str>, chunked: bool) -> Res {
    let mut r = guest_req(code, key);
    r.headers.push(("content-type", "image/jpeg".into()));
    if !chunked {
        r.headers.push(("content-length", file.len().to_string()));
    }
    if let Some(c) = caption {
        r.headers.push(("x-caption", percent_encode(c)));
    }
    send(&t.guest, "POST", &format!("/api/pocket/{code}/photo"), B::Raw(file.to_vec()), &r).await
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
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn host_act(t: &T, code: &str, name: &str, body: Value) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/action"), B::Json(json!({ "name": name, "body": body })), &person()).await
}

async fn guest_act(t: &T, code: &str, key: &str, name: &str, body: Value) -> Res {
    tokio::time::sleep(std::time::Duration::from_millis(110)).await;
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), B::Json(json!({ "name": name, "body": body })), &guest_req(code, key)).await
}

async fn view_of(t: &T, code: &str, key: &str) -> Value {
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), B::None, &guest_req(code, key)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

async fn get_photo(t: &T, code: &str, id: &str, r: &Req) -> Res {
    send(&t.guest, "GET", &format!("/api/pocket/{code}/photo/{id}"), B::None, r).await
}

/// A screen link signed in: the screen's cookie as a request.
async fn screen(t: &T, code: &str) -> Req {
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/link"), B::Json(json!({ "role": "screen" })), &person()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let tok = r.body["token"].as_str().unwrap().to_string();
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/signin"), B::Json(json!({ "token": tok })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    Req { cookies: vec![(guest::host_cookie(code), r.cookie(&guest::host_cookie(code)).unwrap())], ..Default::default() }
}

fn wall_dir(t: &T, code: &str) -> PathBuf {
    t.data.join("pocket").join(code).join("wall")
}

fn files(t: &T, code: &str) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = match std::fs::read_dir(wall_dir(t, code)) {
        Ok(d) => d.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect(),
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
    for bad in [&b"CANARY"[..], b"Exif\0", b"xmpmeta", b"Photoshop 3.0", b"XMP ", b"EXIF"] {
        assert!(!has(b, bad), "{what} still has {:?}", String::from_utf8_lossy(bad));
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
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
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
    p.extend(chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta>ITXT-CANARY</x:xmpmeta>"));
    p.extend(chunk(b"zTXt", b"Author\0\0ZTXT-CANARY"));
    p.extend(chunk(b"tIME", &[0x07, 0xEA, 11, 24, 21, 14, 0]));
    p.extend(chunk(b"IDAT", &[0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01]));
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
    body.extend(riff_chunk(b"VP8L", &[0x2F, 0x1F, 0xC0, 0x95, 0x00, 0x07, 0x10, 0x11]));
    body.extend(riff_chunk(b"EXIF", &tiff(8)));
    body.extend(riff_chunk(b"XMP ", b"<x:xmpmeta>WEBP-XMP-CANARY</x:xmpmeta>"));
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

// ---- Location data ----

#[tokio::test]
async fn location_and_camera_details_never_reach_the_disk_the_wall_or_the_zip() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (_, a) = joined(&t, &code, "Mia Rossi").await;
    let (_, b) = joined(&t, &code, "Sam").await;
    let mut ids = Vec::new();
    for (file, w, h, rotate) in [(jpeg(), 640, 480, 90), (png(), 3, 2, 180), (webp(), 800, 600, 270)] {
        let r = upload(&t, &code, &a, &file).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        let id = r.body["photo"]["id"].as_str().unwrap().to_string();
        let mine = r.body["view"]["mine"].as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).unwrap().clone();
        assert_eq!((mine["w"].as_u64(), mine["h"].as_u64(), mine["rotate"].as_u64()), (Some(w), Some(h), Some(rotate)), "the turn and size are kept: {mine}");
        assert_eq!(mine["status"], "pending");
        ids.push(id);
    }
    // On disk: three files, none with a canary or a metadata block
    let stored = files(&t, &code);
    assert_eq!(stored.len(), 3, "{:?}", stored.iter().map(|f| &f.0).collect::<Vec<_>>());
    for (name, bytes) in &stored {
        assert_clean(name, bytes);
    }
    let jpg = &stored.iter().find(|f| f.0.ends_with(".jpg")).unwrap().1;
    assert!(jpg.ends_with(&[0xFF, 0xD9]) && has(jpg, b"JFIF"), "the JPEG still ends at its end marker and keeps JFIF");
    let wp = &stored.iter().find(|f| f.0.ends_with(".webp")).unwrap().1;
    assert_eq!(wp[20] & 0x0C, 0, "VP8X no longer says EXIF or XMP");
    assert_eq!(u32::from_le_bytes([wp[4], wp[5], wp[6], wp[7]]) as usize, wp.len() - 8, "the RIFF size is right");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(wall_dir(&t, &code)).unwrap().permissions().mode() & 0o777, 0o700);
        for (name, _) in &stored {
            assert_eq!(std::fs::metadata(wall_dir(&t, &code).join(name)).unwrap().permissions().mode() & 0o777, 0o600, "{name}");
        }
    }
    // Served to another guest once approved: the same clean bytes
    for id in &ids {
        assert_eq!(host_act(&t, &code, "approve", json!({ "id": id })).await.status, StatusCode::OK);
        let r = get_photo(&t, &code, id, &guest_req(&code, &b)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_clean("served", &r.bytes);
        assert!(stored.iter().any(|(_, f)| f[..] == r.bytes[..]), "served bytes are the stored file");
    }
    // The zip: the JPEG gets back an Exif block with the orientation (6, 90°) and no other tag,
    // right after JFIF; PNG and WebP come as stored. Without that block, all three are clean.
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}/wall.zip"), B::None, &person()).await;
    assert_eq!(r.status, StatusCode::OK);
    let turn = seg(0xE1, b"Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0");
    assert_clean("the zip", &without(&r.bytes, &turn));
    let entries = unzip(&r.bytes);
    assert_eq!(entries.len(), 3);
    for (name, bytes) in &entries {
        if name.ends_with(".jpg") {
            assert_eq!(&bytes[2..4], &[0xFF, 0xE0], "JFIF still leads");
            let jfif_end = 4 + u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
            assert_eq!(&bytes[jfif_end..jfif_end + turn.len()], &turn[..], "{name}: the orientation block follows JFIF");
            assert_eq!(without(bytes, &turn), *jpg, "{name}: otherwise the stored bytes");
        } else {
            assert!(stored.iter().any(|(_, f)| f == bytes), "{name}: PNG and WebP as stored");
        }
        assert_clean(name, &without(bytes, &turn));
    }
}

/// `hay` with every `needle` taken out.
fn without(hay: &[u8], needle: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(needle) {
            i += needle.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// pa14 QA F1 (Rigel's repro in #1140): PNG and WebP keep only the chunks on their lists, so a
/// C2PA block or a maker's own chunk never reaches the disk, and the WebP flags match what's left.
/// ICC colour profiles stay on purpose.
#[tokio::test]
async fn png_and_webp_keep_only_their_listed_chunks() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
    p.extend(chunk(b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0]));
    p.extend(chunk(b"iCCP", b"ICC-KEPT\0\0x"));
    p.extend(chunk(b"caBX", b"jumb-C2PA-GPS-CANARY 46.0569N 14.5058E"));
    p.extend(chunk(b"prVt", b"PRIVATE-CANARY"));
    p.extend(chunk(b"IDAT", &[0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01]));
    p.extend(chunk(b"IEND", &[]));
    let r = upload(&t, &code, &a, &p).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    // VP8X says ICC, alpha, EXIF, XMP; the file has ICC, the image and a C2PA chunk
    let mut vp8x = vec![0x20 | 0x10 | 0x08 | 0x04, 0, 0, 0];
    vp8x.extend(&799u32.to_le_bytes()[..3]);
    vp8x.extend(&599u32.to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &vp8x));
    body.extend(riff_chunk(b"ICCP", b"ICC-KEPT"));
    body.extend(riff_chunk(b"VP8L", &[0x2F, 0x1F, 0xC0, 0x95, 0x00, 0x07, 0x10, 0x11]));
    body.extend(riff_chunk(b"C2PA", b"WEBP-C2PA-GPS-CANARY 46.0569N"));
    let mut w = b"RIFF".to_vec();
    w.extend((body.len() as u32).to_le_bytes());
    w.extend(body);
    let r = upload(&t, &code, &a, &w).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let stored = files(&t, &code);
    assert_eq!(stored.len(), 2);
    for (name, bytes) in &stored {
        assert!(!has(bytes, b"CANARY") && !has(bytes, b"caBX") && !has(bytes, b"prVt") && !has(bytes, b"C2PA"), "{name} kept an unlisted chunk");
        assert!(has(bytes, b"ICC-KEPT"), "{name}: ICC stays on purpose");
    }
    let wp = &stored.iter().find(|f| f.0.ends_with(".webp")).unwrap().1;
    assert_eq!(wp[20], 0x20 | 0x10, "VP8X says ICC and alpha only");
    assert_eq!(u32::from_le_bytes([wp[4], wp[5], wp[6], wp[7]]) as usize, wp.len() - 8, "the RIFF size is right");
}

// ---- Who sees what ----

#[tokio::test]
async fn a_waiting_photo_is_its_senders_and_the_hosts_and_a_hidden_one_is_nobodys() {
    let t = setup().await;
    let (code, host_key) = make(&t, spec(json!({}))).await;
    let (a_id, a) = joined(&t, &code, "Mia Rossi").await;
    let (_, b) = joined(&t, &code, "Sam").await;
    let tv = screen(&t, &code).await;
    let id = sent(&t, &code, &a, &jpeg()).await;
    let stranger = Req::default();
    let host = host_req(&host_key);

    let status = |r: Res| r.status;
    assert_eq!(status(get_photo(&t, &code, &id, &stranger).await), StatusCode::UNAUTHORIZED);
    let r = get_photo(&t, &code, &id, &guest_req(&code, &b)).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_such_photo")), "B can't tell it exists");
    assert_eq!(status(get_photo(&t, &code, &id, &tv).await), StatusCode::NOT_FOUND, "the TV shows approved only");
    let r = get_photo(&t, &code, &id, &guest_req(&code, &a)).await;
    assert_eq!((r.status, r.header("cache-control")), (StatusCode::OK, Some("private, no-store")));
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(status(get_photo(&t, &code, &id, &host).await), StatusCode::OK);
    let made_up = "ph_AAAAAAAAAAAAAAAA";
    assert_eq!(status(get_photo(&t, &code, made_up, &guest_req(&code, &a)).await), StatusCode::NOT_FOUND);
    assert_eq!(status(get_photo(&t, &code, "../../etc", &guest_req(&code, &a)).await), StatusCode::NOT_FOUND);

    let bv = view_of(&t, &code, &b).await;
    assert_eq!(bv["view"]["wall"], json!([]), "B's wall is empty");
    assert_eq!(bv["view"]["mine"], json!([]));
    assert!(!bv.to_string().contains(&id) && !bv.to_string().contains(&a_id), "B's view names neither A's photo nor A");
    let av = view_of(&t, &code, &a).await;
    assert_eq!(av["view"]["mine"][0]["id"], id.as_str());
    assert_eq!(av["view"]["limits"]["mine_count"], 1);
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), B::None, &tv).await;
    assert_eq!(r.body["view"]["photos"], json!([]), "{}", r.body);
    assert_eq!(r.body["view"]["host_name"], "Oleks", "TV3 names the host");
    let info = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), B::None, &stranger).await.body;
    assert_eq!((info["view"]["limits"]["photo_cap_mb"].clone(), info["view"]["limits"]["per_guest_max"].clone()), (json!(15), json!(40)), "{info}");
    assert!(info["view"]["limits"].get("mine_count").is_none() && !info.to_string().contains(&a_id), "seatless: no counts, no player ids");

    // Approved: everyone in the pocket, cached with a check
    assert_eq!(host_act(&t, &code, "approve", json!({ "id": id })).await.status, StatusCode::OK);
    let r = get_photo(&t, &code, &id, &guest_req(&code, &b)).await;
    assert_eq!((r.status, r.header("cache-control")), (StatusCode::OK, Some("private, no-cache")));
    let etag = r.header("etag").unwrap().to_string();
    let mut again = guest_req(&code, &b);
    again.headers.push(("if-none-match", etag));
    assert_eq!(status(get_photo(&t, &code, &id, &again).await), StatusCode::NOT_MODIFIED);
    assert_eq!(status(get_photo(&t, &code, &id, &tv).await), StatusCode::OK);
    assert_eq!(status(get_photo(&t, &code, &id, &stranger).await), StatusCode::UNAUTHORIZED);
    let bv = view_of(&t, &code, &b).await;
    assert_eq!(bv["view"]["wall"][0]["by_name"], "Mia", "first name only");
    assert!(!bv.to_string().contains(&a_id), "still no player id for other guests");
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), B::None, &tv).await;
    assert_eq!(r.body["view"]["photos"][0]["id"], id.as_str());
    assert!(!r.body["view"].to_string().contains(&a_id));

    // Hidden: gone for everyone, and off the disk
    assert_eq!(host_act(&t, &code, "hide", json!({ "id": id })).await.status, StatusCode::OK);
    for r in [guest_req(&code, &a), guest_req(&code, &b), tv.clone(), host.clone()] {
        assert_eq!(status(get_photo(&t, &code, &id, &r).await), StatusCode::NOT_FOUND);
    }
    assert!(files(&t, &code).is_empty(), "the file went at once");
    assert_eq!(view_of(&t, &code, &a).await["view"]["mine"], json!([]), "the sender's own hidden photo drops quietly");
    assert_eq!(view_of(&t, &code, &a).await["view"]["limits"]["mine_count"], 1, "a hidden photo still counts towards the guest's max");

    // Only the host approves or hides; guests may only edit their own captions
    let id2 = sent(&t, &code, &a, &jpeg()).await;
    let r = guest_act(&t, &code, &a, "approve", json!({ "id": id2 })).await;
    assert_ne!(r.status, StatusCode::OK);
    assert_eq!(view_of(&t, &code, &b).await["view"]["wall"], json!([]));
    // A wall photo URL on another kind of pocket is a 404
    let r = send(&t.main, "POST", "/api/pockets", B::Json(json!({ "kit": "test", "spec": { "title": "Quiz night" } })), &person()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let q = r.body["pocket"]["code"].as_str().unwrap().to_string();
    let r = join(&t, &q, "Ana").await;
    let qk = r.cookie(&guest::guest_cookie(&q)).unwrap();
    assert_eq!(upload(&t, &q, &qk, &jpeg()).await.status, StatusCode::NOT_FOUND);
    assert_eq!(status(get_photo(&t, &q, &id2, &guest_req(&q, &qk)).await), StatusCode::NOT_FOUND);
    assert_eq!(send(&t.main, "GET", &format!("/api/pockets/{q}/wall.zip"), B::None, &person()).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_persons_card_reads_photos_on_the_main_port() {
    let t = setup().await;
    let (code, host_key) = make(&t, spec(json!({}))).await;
    let (_, a) = joined(&t, &code, "Mia Rossi").await;
    let id = sent(&t, &code, &a, &jpeg()).await;
    let main_photo = |id: String, r: Req| {
        let t = &t;
        let code = code.clone();
        async move { send(&t.main, "GET", &format!("/api/pockets/{code}/photo/{id}"), B::None, &r).await }
    };

    // Waiting: the person sees it (the card's queue), never kept, same bytes as the host's
    let r = main_photo(id.clone(), person()).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(r.header("cache-control"), Some("private, no-store"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("content-security-policy"), Some("default-src 'none'; sandbox"));
    assert_eq!(r.header("cross-origin-resource-policy"), Some("same-origin"));
    let on_guest_port = get_photo(&t, &code, &id, &host_req(&host_key)).await;
    assert_eq!((on_guest_port.status, &r.bytes), (StatusCode::OK, &on_guest_port.bytes));

    // Person-only: the API token from elsewhere, never a pocket key, and not on the guest port
    assert_eq!(main_photo(id.clone(), Req { peer: Some(REMOTE), ..Default::default() }).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(main_photo(id.clone(), Req { peer: Some(REMOTE), bearer: Some(TOKEN.into()), ..Default::default() }).await.status, StatusCode::OK);
    assert_eq!(main_photo(id.clone(), Req { peer: Some(REMOTE), bearer: Some(host_key.clone()), ..Default::default() }).await.status, StatusCode::UNAUTHORIZED);
    let r = main_photo(id.clone(), Req { peer: Some(REMOTE), ..guest_req(&code, &a) }).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "a guest's cookie means nothing here");
    for r in [guest_req(&code, &a), host_req(&host_key)] {
        let r = send(&t.guest, "GET", &format!("/api/pockets/{code}/photo/{id}"), B::None, &r).await;
        assert!(r.status.is_client_error(), "{}", r.status);
    }

    // Made-up and path-like ids are 404s
    for bad in ["ph_AAAAAAAAAAAAAAAA", "..%2F..%2Fetc", "ph_1.jpg"] {
        let r = main_photo(bad.to_string(), person()).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_such_photo")), "{bad}");
    }

    // The summary carries pa14b's fields: nothing picking, and a best set is the host's own
    let summary = || {
        let t = &t;
        let code = code.clone();
        async move { send(&t.main, "GET", &format!("/api/pockets/{code}"), B::None, &person()).await.body["summary"].clone() }
    };
    let s = summary().await;
    assert_eq!((s["picking"].clone(), s["best_by"].clone()), (json!(false), Value::Null), "{s}");

    // Approved: kept with a check, and the check works here too
    assert_eq!(host_act(&t, &code, "approve", json!({ "id": id })).await.status, StatusCode::OK);
    let r = main_photo(id.clone(), person()).await;
    assert_eq!((r.status, r.header("cache-control")), (StatusCode::OK, Some("private, no-cache")));
    let mut again = person();
    again.headers.push(("if-none-match", r.header("etag").unwrap().to_string()));
    let r = main_photo(id.clone(), again).await;
    assert_eq!((r.status, r.header("x-content-type-options")), (StatusCode::NOT_MODIFIED, Some("nosniff")));
    assert_eq!(host_act(&t, &code, "set_best", json!({ "ids": [id] })).await.status, StatusCode::OK);
    let s = summary().await;
    assert_eq!((s["picking"].clone(), s["best_by"].clone(), s["best"].clone()), (json!(false), json!("host"), json!([id])), "{s}");

    // Hidden: gone here too
    assert_eq!(host_act(&t, &code, "hide", json!({ "id": id })).await.status, StatusCode::OK);
    assert_eq!(main_photo(id.clone(), person()).await.status, StatusCode::NOT_FOUND);

    // Another kind of pocket has no photos
    let r = send(&t.main, "POST", "/api/pockets", B::Json(json!({ "kit": "test", "spec": { "title": "Quiz night" } })), &person()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let q = r.body["pocket"]["code"].as_str().unwrap().to_string();
    assert_eq!(send(&t.main, "GET", &format!("/api/pockets/{q}/photo/{id}"), B::None, &person()).await.status, StatusCode::NOT_FOUND);
}

// ---- Limits ----

#[tokio::test]
async fn limits_refuse_before_anything_is_stored() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({ "photo_cap_mb": 1, "pocket_cap_mb": 1, "per_guest_max": 3 }))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    let (_, c) = joined(&t, &code, "Cleo").await;

    // Over the photo cap, declared or not
    let big = jpeg_padded(1024 * 1024 + 10);
    for chunked in [false, true] {
        let r = upload_as(&t, &code, &a, &big, None, chunked).await;
        assert_eq!((r.status, r.body["error"].as_str(), r.body["limit"].as_u64()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_big"), Some(1)), "chunked {chunked}: {}", r.body);
    }
    // Not photos
    let r = upload(&t, &code, &a, b"just some text, not a photo").await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNSUPPORTED_MEDIA_TYPE, Some("not_photo")));
    let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
    let r = upload(&t, &code, &a, &heic).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNSUPPORTED_MEDIA_TYPE, Some("heic")));
    let cut = jpeg()[..60].to_vec();
    let r = upload(&t, &code, &a, &cut).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNSUPPORTED_MEDIA_TYPE, Some("broken_photo")));
    assert!(files(&t, &code).is_empty(), "nothing refused was stored");

    // Per guest: 3, the 4th is refused (counts sends, so hiding doesn't free a slot)
    let mid = jpeg_padded(300 * 1024);
    for _ in 0..3 {
        sent(&t, &code, &a, &mid).await;
    }
    let r = upload(&t, &code, &a, &jpeg()).await;
    assert_eq!((r.status, r.body["error"].as_str(), r.body["limit"].as_u64()), (StatusCode::CONFLICT, Some("guest_max"), Some(3)));
    // The wall's cap: 3 x 300 KB are in, another 300 KB would pass 1 MB
    let r = upload(&t, &code, &b, &mid).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::INSUFFICIENT_STORAGE, Some("wall_full")));
    // The disk floor: under 2 GB free stops uploads
    t.state.pocket.set_fake_free_disk(Some(2 * 1024 * 1024 * 1024));
    let r = upload(&t, &code, &b, &jpeg()).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::INSUFFICIENT_STORAGE, Some("disk_low")));
    t.state.pocket.set_fake_free_disk(Some(50 * 1024 * 1024 * 1024));
    sent(&t, &code, &b, &jpeg()).await;
    assert_eq!(files(&t, &code).len(), 4, "only the accepted photos are on disk");

    // Signed out
    let r = upload(&t, &code, "not-a-key", &jpeg()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // Rate: a burst of 10, then the 11th waits (with Retry-After)
    let (code2, _) = make(&t, spec(json!({}))).await;
    let (_, d) = joined(&t, &code2, "Dan").await;
    for n in 0..10 {
        assert_eq!(upload(&t, &code2, &d, &jpeg()).await.status, StatusCode::CREATED, "photo {n}");
    }
    let r = upload(&t, &code2, &d, &jpeg()).await;
    assert_eq!((r.status, r.body["error"].as_str(), r.header("retry-after")), (StatusCode::TOO_MANY_REQUESTS, Some("slow_down"), Some("3")));
    assert_eq!(files(&t, &code2).len(), 10);

    // The 16 KB limit still holds on every other guest route
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), B::Json(json!({ "name": "x".repeat(17 * 1024), "pin": PIN })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    let r = guest_act(&t, &code, &c, "caption", json!({ "id": "ph_x", "caption": "y".repeat(17 * 1024) })).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Uploads stop at closes_at, though the wall stays up
    let now = Utc::now();
    let (code3, _) = make(&t, spec(json!({ "closes_at": (now + Duration::hours(1)).to_rfc3339() }))).await;
    let (_, e) = joined(&t, &code3, "Eve").await;
    sent(&t, &code3, &e, &jpeg()).await;
    pocket::sweep(&t.state, now + Duration::hours(2)).await;
    let r = upload(&t, &code3, &e, &jpeg()).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::GONE, Some("closed")));
    let v = view_of(&t, &code3, &e).await;
    assert_eq!(v["view"]["state"], "closed");
    assert_eq!(files(&t, &code3).len(), 1);
}

// ---- Captions ----

#[tokio::test]
async fn captions_are_cut_filtered_and_editable_until_approved() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let r = upload_as(&t, &code, &a, &jpeg(), Some("First dance 💃 at the barn"), false).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let id = r.body["photo"]["id"].as_str().unwrap().to_string();
    assert_eq!(r.body["view"]["mine"][0]["caption"], "First dance 💃 at the barn");
    let long = "x".repeat(200);
    let r = upload_as(&t, &code, &a, &jpeg(), Some(&long), false).await;
    assert_eq!(r.body["view"]["mine"][0]["caption"].as_str().map(|c| c.chars().count()), Some(80));
    let r = upload_as(&t, &code, &a, &jpeg(), Some("what the fuck"), false).await;
    assert_eq!(r.status, StatusCode::CREATED, "a caption never stops a photo");
    assert_eq!((r.body["view"]["mine"][0]["caption"].clone(), r.body["view"]["mine"][0]["caption_removed"].clone()), (Value::Null, json!(true)));
    // Editable while waiting, then fixed
    assert_eq!(guest_act(&t, &code, &a, "caption", json!({ "id": id, "caption": "Barn dance" })).await.status, StatusCode::OK);
    assert_eq!(host_act(&t, &code, "approve", json!({ "id": id })).await.status, StatusCode::OK);
    assert_eq!(guest_act(&t, &code, &a, "caption", json!({ "id": id, "caption": "Later" })).await.status, StatusCode::CONFLICT);
    let v = view_of(&t, &code, &a).await;
    assert_eq!(v["view"]["wall"][0]["caption"], "Barn dance");
    // A wall without captions keeps none
    let (code2, _) = make(&t, spec(json!({ "captions": false }))).await;
    let (_, b) = joined(&t, &code2, "Ben").await;
    let r = upload_as(&t, &code2, &b, &jpeg(), Some("hello"), false).await;
    assert_eq!((r.status, r.body["view"]["mine"][0]["caption"].clone()), (StatusCode::CREATED, Value::Null));
}

// ---- Zip ----

#[tokio::test]
async fn the_zip_is_the_persons_and_holds_what_they_asked_for() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (a_id, a) = joined(&t, &code, "Mia Rossi").await;
    let (_, b) = joined(&t, &code, "Sam").await;
    let p1 = sent(&t, &code, &a, &jpeg()).await;
    let p2 = sent(&t, &code, &b, &png()).await;
    let _waiting = sent(&t, &code, &b, &webp()).await;
    let gone = sent(&t, &code, &a, &jpeg()).await;
    host_act(&t, &code, "approve_all_from", json!({ "guest": a_id })).await;
    host_act(&t, &code, "approve", json!({ "id": p2 })).await;
    host_act(&t, &code, "hide", json!({ "id": gone })).await;
    assert_eq!(host_act(&t, &code, "set_best", json!({ "ids": [p1] })).await.status, StatusCode::OK);
    let zip = |q: &'static str| {
        let t = &t;
        let code = code.clone();
        async move { send(&t.main, "GET", &format!("/api/pockets/{code}/wall.zip{q}"), B::None, &person()).await }
    };
    let r = zip("").await;
    assert_eq!(r.header("content-type"), Some("application/zip"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("content-disposition"), Some(format!("attachment; filename=\"wall-{code}.zip\"").as_str()));
    let names: Vec<String> = unzip(&r.bytes).into_iter().map(|e| e.0).collect();
    assert_eq!(names.len(), 2, "approved only: {names:?}");
    assert!(names[0].starts_with("001-mia-") && names[0].ends_with(".jpg"), "{names:?}");
    assert!(names[1].starts_with("002-sam-") && names[1].ends_with(".png"), "{names:?}");
    assert_eq!(unzip(&zip("?all=1").await.bytes).len(), 3, "with the waiting one, never the hidden one");
    let best = unzip(&zip("?best=1").await.bytes);
    assert_eq!(best.len(), 1);
    assert!(best[0].0.starts_with("001-mia-"));
    // Guests see best only once the host showed them
    let v = view_of(&t, &code, &b).await;
    assert_eq!(v["view"]["best_set"], true);
    assert_eq!(v["view"]["wall"].as_array().unwrap().iter().filter(|p| p["best"] == true).count(), 1);
    // Person-only: no token from elsewhere, and nothing on the guest port
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}/wall.zip"), B::None, &Req { peer: Some(REMOTE), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}/wall.zip"), B::None, &Req { peer: Some(REMOTE), bearer: Some(TOKEN.into()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK);
    for path in [format!("/api/pockets/{code}/wall.zip"), format!("/api/pocket/{code}/wall.zip")] {
        let r = send(&t.guest, "GET", &path, B::None, &guest_req(&code, &a)).await;
        assert!(r.status.is_client_error(), "{path}: {}", r.status);
    }
}

// ---- Retention ----

#[tokio::test]
async fn photos_go_thirty_days_after_the_end_and_at_once_on_delete() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let id = sent(&t, &code, &a, &jpeg()).await;
    host_act(&t, &code, "approve", json!({ "id": id })).await;
    sent(&t, &code, &a, &png()).await;
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), B::None, &person()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), B::None, &person()).await;
    let ended: DateTime<Utc> = r.body["pocket"]["ended_at"].as_str().unwrap().parse().unwrap();
    let expires: DateTime<Utc> = r.body["pocket"]["expires_at"].as_str().unwrap().parse().unwrap();
    assert_eq!(expires, ended + Duration::days(30));
    // Ended: no more photos, the zip still works
    assert_eq!(upload(&t, &code, &a, &jpeg()).await.status, StatusCode::GONE);
    assert_eq!(send(&t.main, "GET", &format!("/api/pockets/{code}/wall.zip"), B::None, &person()).await.status, StatusCode::OK);
    assert_eq!(pocket::sweep(&t.state, expires - Duration::minutes(1)).await.deleted, 0, "not at +29 days 23:59");
    assert_eq!(files(&t, &code).len(), 2);
    assert_eq!(pocket::sweep(&t.state, expires + Duration::minutes(1)).await.deleted, 1);
    assert!(!t.data.join("pocket").join(&code).exists(), "the photo folder went with the pocket");
    assert!(t.state.db.pocket_get(&code).await.unwrap().is_none());

    // Deleting by hand takes the photos at once
    let (code2, _) = make(&t, spec(json!({}))).await;
    let (_, b) = joined(&t, &code2, "Ben").await;
    sent(&t, &code2, &b, &jpeg()).await;
    assert_eq!(files(&t, &code2).len(), 1);
    let r = send(&t.main, "DELETE", &format!("/api/pockets/{code2}"), B::None, &person()).await;
    assert!(r.status.is_success(), "{}", r.status);
    assert!(!t.data.join("pocket").join(&code2).exists());
}
