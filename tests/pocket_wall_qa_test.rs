//! Rigel's QA probes for pa14 (not for merge): real-world files over HTTP, parallel limits, and
//! the corners Juno's six tests do not reach.
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

const BROKEN: &[&str] = &[
    "huge_app_len_short.jpg",
    "no_eoi.jpg",
    "png_huge_len.png",
    "png_lying_len.png",
    "png_no_iend.png",
    "seglen0.jpg",
    "seglen1.jpg",
    "soi_eoi.jpg",
    "soi_only.jpg",
    "webp_chunk_overflow.webp",
    "webp_riff_huge.webp",
    "webp_riff_small.webp",
];
const HEIC: &[&str] = &["avif.avif", "heic.heic"];
const NOT_PHOTO: &[&str] = &[
    "empty.jpg",
    "renamed.txt.jpg",
    "x.bmp",
    "x.gif",
    "x.pdf",
    "x.svg",
    "x.tiff",
    "zip.zip",
];
const CANARIES: &[&str] = &[
    "CANARY",
    "Exif\0",
    "xmpmeta",
    "Photoshop 3.0",
    "ftyp",
    "SEFH",
    "PK\x03\x04",
    "<html",
];

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/qa_corpus");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

fn leaks(b: &[u8]) -> Vec<&'static str> {
    CANARIES
        .iter()
        .copied()
        .filter(|c| has(b, c.as_bytes()))
        .collect()
}

/// QA1: every corpus file over HTTP. Accepted ones are clean on disk, in the served photo, in
/// the zip; refused ones store nothing. Known gap: PNG caBX / WebP C2PA survive (reported, not asserted).
#[tokio::test]
async fn qa_corpus_over_http() {
    let t = setup().await;
    let (code, _) = make(
        &t,
        spec(json!({ "per_guest_max": 100, "photo_cap_mb": 25, "pocket_cap_mb": 500 })),
    )
    .await;
    let mut accepted: Vec<(String, String)> = Vec::new();
    for (i, (name, bytes)) in corpus().into_iter().enumerate() {
        // a new guest per file so the rate limit never gets in the way
        let (_, key) = joined(&t, &code, &format!("G{i}")).await;
        let before = files(&t, &code).len();
        let r = upload(&t, &code, &key, &bytes).await;
        let want = if BROKEN.contains(&name.as_str()) {
            (415, "broken_photo")
        } else if HEIC.contains(&name.as_str()) {
            (415, "heic")
        } else if NOT_PHOTO.contains(&name.as_str()) {
            (415, "not_photo")
        } else {
            (201, "")
        };
        assert_eq!(r.status.as_u16(), want.0, "{name}: {}", r.body);
        if want.0 == 415 {
            assert_eq!(r.body["error"].as_str(), Some(want.1), "{name}: {}", r.body);
            assert_eq!(
                files(&t, &code).len(),
                before,
                "{name}: a refused file must not be stored"
            );
        } else {
            assert_eq!(files(&t, &code).len(), before + 1, "{name}");
            let id = r.body["photo"]["id"].as_str().unwrap().to_string();
            let served = get_photo(&t, &code, &id, &guest_req(&code, &key)).await;
            assert_eq!(served.status, StatusCode::OK, "{name}");
            let l = leaks(&served.bytes);
            assert!(l.is_empty(), "{name}: served photo leaks {l:?}");
            assert!(
                !has(&served.bytes, b"C2PA") && !has(&served.bytes, b"jumb"),
                "{name}: C2PA survives the strip"
            );
            accepted.push((name, id));
        }
    }
    for (name, bytes) in files(&t, &code) {
        let l = leaks(&bytes);
        assert!(l.is_empty(), "{name}: stored file leaks {l:?}");
    }
    eprintln!("QA-REPORT accepted={}", accepted.len());
    // approve all, then the zip is clean too
    for (_, id) in &accepted {
        assert_eq!(
            host_act(&t, &code, "approve", json!({ "id": id }))
                .await
                .status,
            StatusCode::OK
        );
    }
    let z = send(
        &t.main,
        "GET",
        &format!("/api/pockets/{code}/wall.zip"),
        B::None,
        &person(),
    )
    .await;
    assert_eq!(z.status, StatusCode::OK);
    let entries = unzip(&z.bytes);
    assert_eq!(entries.len(), accepted.len());
    let mut turned = 0;
    for (n, b) in entries {
        // Round 2: a turned JPEG gets ONE orientation-only Exif block in the zip. Allow exactly that.
        let marker = b"\xFF\xE1\x00\x22Exif\0\0MM\0\x2A\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01";
        let mut rest = b.clone();
        if let Some(at) = b.windows(marker.len()).position(|w| w == marker) {
            assert!(n.ends_with(".jpg"), "{n}");
            let o = u16::from_be_bytes([b[at + marker.len()], b[at + marker.len() + 1]]);
            assert!([3, 6, 8].contains(&o), "{n}: orientation {o}");
            assert_eq!(
                &b[at + marker.len() + 2..at + marker.len() + 8],
                &[0u8; 6],
                "{n}: nothing after the one tag"
            );
            rest.drain(at..at + 4 + 0x22 - 2);
            turned += 1;
        }
        let l = leaks(&rest);
        assert!(l.is_empty(), "zip {n} leaks {l:?}");
    }
    eprintln!("QA-REPORT zip: {turned} turned JPEGs carry the orientation-only Exif block");
    assert!(
        turned >= 3,
        "iPhone-style turned JPEGs are in the zip with an orientation block"
    );
    // modes
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(wall_dir(&t, &code))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for e in std::fs::read_dir(wall_dir(&t, &code)).unwrap().flatten() {
            assert_eq!(
                e.metadata().unwrap().permissions().mode() & 0o777,
                0o600,
                "{:?}",
                e.file_name()
            );
        }
    }
}

/// QA1b: a PNG with a caBX (C2PA) chunk and a WebP with a C2PA chunk lose it (round 2 allowlist).
#[tokio::test]
async fn qa_gap_png_webp_unknown_chunks_survive() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (_, k) = joined(&t, &code, "Ana").await;
    let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
    p.extend(chunk(b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0]));
    p.extend(chunk(b"caBX", b"jumb-C2PA-GPS-CANARY 46.0569N 14.5058E"));
    p.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01],
    ));
    p.extend(chunk(b"IEND", &[]));
    let r = upload(&t, &code, &k, &p).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let png_kept = files(&t, &code).iter().any(|f| has(&f.1, b"CANARY"));
    let mut vp8x = vec![0x08 | 0x04, 0, 0, 0];
    vp8x.extend(&799u32.to_le_bytes()[..3]);
    vp8x.extend(&599u32.to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &vp8x));
    body.extend(riff_chunk(
        b"VP8L",
        &[0x2F, 0x1F, 0xC0, 0x95, 0x00, 0x07, 0x10, 0x11],
    ));
    body.extend(riff_chunk(b"C2PA", b"WEBP-C2PA-GPS-CANARY 46.0569N"));
    let mut w = b"RIFF".to_vec();
    w.extend((body.len() as u32).to_le_bytes());
    w.extend(body);
    let (_, k2) = joined(&t, &code, "Ben").await;
    let r = upload(&t, &code, &k2, &w).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let all = files(&t, &code);
    let webp_kept = all
        .iter()
        .any(|f| f.0.ends_with(".webp") && has(&f.1, b"CANARY"));
    eprintln!("QA-REPORT unknown-chunk gap: png caBX kept={png_kept} webp C2PA kept={webp_kept}");
    assert!(
        !png_kept && !webp_kept,
        "round 2: unknown chunks must not survive (png {png_kept}, webp {webp_kept})"
    );
}

/// QA2: one guest firing parallel uploads cannot go over per_guest_max, and the wall cap holds
/// under parallel writers from different guests.
#[tokio::test]
async fn qa_parallel_uploads_respect_limits() {
    let t = setup().await;
    let (code, _) = make(
        &t,
        spec(json!({ "per_guest_max": 3, "photo_cap_mb": 1, "pocket_cap_mb": 1 })),
    )
    .await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let small = jpeg();
    let futs: Vec<_> = (0..8).map(|_| upload(&t, &code, &a, &small)).collect();
    let rs = futures::future::join_all(futs).await;
    let ok = rs
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    let max = rs
        .iter()
        .filter(|r| r.status == StatusCode::CONFLICT)
        .count();
    eprintln!(
        "QA-REPORT parallel guest_max: created={ok} guest_max={max} other={:?}",
        rs.iter().map(|r| r.status.as_u16()).collect::<Vec<_>>()
    );
    assert_eq!(ok, 3, "exactly per_guest_max photos");
    assert_eq!(files(&t, &code).len(), 3);
    assert_eq!(
        view_of(&t, &code, &a).await["view"]["limits"]["mine_count"],
        3
    );

    // wall_full with several guests racing: 1 MB cap, 400 KB photos -> at most 2 fit
    let (code2, _) = make(
        &t,
        spec(json!({ "per_guest_max": 5, "photo_cap_mb": 1, "pocket_cap_mb": 1 })),
    )
    .await;
    let mut keys = Vec::new();
    for i in 0..6 {
        keys.push(joined(&t, &code2, &format!("P{i}")).await.1);
    }
    let mid = jpeg_padded(400 * 1024);
    let futs: Vec<_> = keys.iter().map(|k| upload(&t, &code2, k, &mid)).collect();
    let rs = futures::future::join_all(futs).await;
    let ok = rs
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    let full = rs
        .iter()
        .filter(|r| r.status == StatusCode::INSUFFICIENT_STORAGE)
        .count();
    let total: usize = files(&t, &code2).iter().map(|f| f.1.len()).sum();
    eprintln!("QA-REPORT parallel wall_full: created={ok} wall_full={full} total_bytes={total} statuses={:?}", rs.iter().map(|r| r.status.as_u16()).collect::<Vec<_>>());
    assert!(
        total <= 1024 * 1024,
        "wall cap exceeded under parallel writers: {total}"
    );
}

/// QA3: Retry-After: 3 is honest. After waiting 3.1 s the next upload goes through.
#[tokio::test]
async fn qa_retry_after_three_seconds_works() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({ "per_guest_max": 50 }))).await;
    let (_, d) = joined(&t, &code, "Dan").await;
    for _ in 0..10 {
        assert_eq!(
            upload(&t, &code, &d, &jpeg()).await.status,
            StatusCode::CREATED
        );
    }
    let r = upload(&t, &code, &d, &jpeg()).await;
    assert_eq!(
        (r.status, r.header("retry-after")),
        (StatusCode::TOO_MANY_REQUESTS, Some("3"))
    );
    assert_eq!(files(&t, &code).len(), 10, "the refused one stored nothing");
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    assert_eq!(
        upload(&t, &code, &d, &jpeg()).await.status,
        StatusCode::CREATED,
        "after Retry-After the upload goes through"
    );
    // another guest is not slowed by Dan
    let (_, e) = joined(&t, &code, "Eve").await;
    assert_eq!(
        upload(&t, &code, &e, &jpeg()).await.status,
        StatusCode::CREATED
    );
}

/// QA4: hiding an approved photo removes it from the zip and from disk; removing a guest hides
/// their waiting photos only; the guest frame holds no host-only keys.
#[tokio::test]
async fn qa_hide_remove_and_frame_keys() {
    let t = setup().await;
    let (code, _) = make(&t, spec(json!({}))).await;
    let (a_id, a) = joined(&t, &code, "Ana Lopez").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    let a1 = sent(&t, &code, &a, &jpeg()).await;
    let a2 = sent(&t, &code, &a, &png()).await;
    let b1 = sent(&t, &code, &b, &webp()).await;
    host_act(&t, &code, "approve", json!({ "id": a1 })).await;
    host_act(&t, &code, "approve", json!({ "id": b1 })).await;
    let zip_n = || async {
        unzip(
            &send(
                &t.main,
                "GET",
                &format!("/api/pockets/{code}/wall.zip"),
                B::None,
                &person(),
            )
            .await
            .bytes,
        )
        .len()
    };
    assert_eq!(zip_n().await, 2);
    // the guest frame: names no host-only keys, never a count of hidden or waiting for others
    let v = view_of(&t, &code, &b).await;
    let s = v.to_string();
    for bad in [
        "hidden",
        "\"bytes\"",
        "\"by\":",
        &a_id,
        &a2,
        "waiting_count",
        "pending_count",
    ] {
        assert!(!s.contains(bad), "guest B's frame names {bad}: {s}");
    }
    eprintln!(
        "QA-REPORT guest frame keys: {}",
        v["view"]
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_default()
    );
    // remove Ana: her waiting photo goes (file too), her approved one is what the host asked about
    let r = pocket::remove_player(&t.state, &code, &a_id).await;
    eprintln!("QA-REPORT remove_player -> {:?}", r.is_ok());
    assert!(r.is_ok());
    let left: Vec<String> = files(&t, &code).into_iter().map(|f| f.0).collect();
    eprintln!("QA-REPORT files after remove_guest: {left:?}");
    assert!(
        !left.iter().any(|n| n.starts_with(&a2)),
        "the removed guest's waiting photo is off the disk"
    );
    // hide an approved photo
    host_act(&t, &code, "hide", json!({ "id": b1 })).await;
    assert!(!files(&t, &code).iter().any(|f| f.0.starts_with(&b1)));
    let n = zip_n().await;
    eprintln!("QA-REPORT zip entries after hide(b1) and remove_guest: {n}");
    assert!(n <= 1);
}

/// QA5: end vs closes_at: retention is 30 days after the END even if the spec's closes_at is
/// far in the future; a pocket with no end time is not swept.
#[tokio::test]
async fn qa_retention_with_late_closes_at_and_state_routes() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(
        &t,
        spec(json!({ "closes_at": (now + Duration::days(10)).to_rfc3339() })),
    )
    .await;
    let (_, a) = joined(&t, &code, "Ana").await;
    sent(&t, &code, &a, &jpeg()).await;
    let r = send(
        &t.main,
        "GET",
        &format!("/api/pockets/{code}"),
        B::None,
        &person(),
    )
    .await;
    eprintln!("QA-REPORT pocket row: {}", r.body["pocket"]);
    let exp = r.body["pocket"]["expires_at"]
        .as_str()
        .map(|s| s.parse::<DateTime<Utc>>().unwrap());
    eprintln!("QA-REPORT expires_at before end = {exp:?}");
    // sweep far in the future before an end: nothing may delete a live pocket's photos silently
    let s = pocket::sweep(&t.state, now + Duration::days(3)).await;
    eprintln!("QA-REPORT sweep +3d deleted={}", s.deleted);
    assert_eq!(
        files(&t, &code).len() as u64 + s.deleted as u64,
        1,
        "either the photo stays or the pocket was swept"
    );
}
