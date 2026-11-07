//! The bots' private browser without Node (0.7.6, O5): Chrome for Testing is downloaded the first
//! time a bot needs a browser and there's no Chrome. Everything here talks to a local fake of
//! Google's build list and storage; nothing is downloaded from the internet.
mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kwnt::sandbox::chrome_download::{self, binary_in, cache_root, ChromeSetup, SetupState, FAILED_LINE};
use kwnt::sandbox::computer::find_chrome;
use serde_json::json;

const VERSION: &str = "131.0.6778.85";

/// A Chrome for Testing zip for linux64 whose `chrome` is a script answering `--version`.
fn build(chrome: &str) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let exe = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        let file = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o644);
        zip.add_directory("chrome-linux64/", exe).unwrap();
        zip.start_file("chrome-linux64/chrome", exe).unwrap();
        zip.write_all(chrome.as_bytes()).unwrap();
        zip.start_file("chrome-linux64/resources.pak", file).unwrap();
        zip.write_all(&vec![7u8; 4096]).unwrap();
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn good_build() -> Vec<u8> {
    build(&format!("#!/bin/sh\necho 'Google Chrome for Testing {VERSION} '\n"))
}

#[derive(Clone)]
struct Fake {
    /// What storage answers: the zip, or an error status.
    zip: Arc<Mutex<Result<Vec<u8>, u16>>>,
    /// Where the build list says the download is (None: on the fake storage).
    url: Arc<Mutex<Option<String>>>,
    base: String,
    list_hits: Arc<AtomicUsize>,
    zip_hits: Arc<AtomicUsize>,
}

impl Fake {
    fn serve(&self, zip: Result<Vec<u8>, u16>) {
        *self.zip.lock().unwrap() = zip;
    }
    fn json(&self) -> String {
        format!("{}/cft.json", self.base)
    }
    fn storage(&self) -> String {
        format!("{}/storage/", self.base)
    }
    fn setup(&self, home: &Path) -> ChromeSetup {
        ChromeSetup::new(cache_root("linux", home), Some("linux64")).source(&self.json(), &self.storage())
    }
}

async fn list(State(f): State<Fake>) -> Json<serde_json::Value> {
    f.list_hits.fetch_add(1, Ordering::SeqCst);
    let url = f
        .url
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| format!("{}{VERSION}/linux64/chrome-linux64.zip", f.storage()));
    Json(json!({
        "timestamp": "2026-11-07T00:00:00.000Z",
        "channels": {
            "Stable": {
                "channel": "Stable",
                "version": VERSION,
                "revision": "1381561",
                "downloads": {
                    "chrome": [
                        {"platform": "mac-arm64", "url": format!("{}{VERSION}/mac-arm64/chrome-mac-arm64.zip", f.storage())},
                        {"platform": "linux64", "url": url},
                    ],
                    "chromedriver": []
                }
            },
            "Beta": {"channel": "Beta", "version": "132.0.0.0", "downloads": {"chrome": []}}
        }
    }))
}

async fn storage(State(f): State<Fake>, uri: Uri) -> Response {
    f.zip_hits.fetch_add(1, Ordering::SeqCst);
    assert!(uri.path().ends_with("/linux64/chrome-linux64.zip"), "{}", uri.path());
    match f.zip.lock().unwrap().clone() {
        Ok(bytes) => bytes.into_response(),
        Err(code) => StatusCode::from_u16(code).unwrap().into_response(),
    }
}

async fn fake() -> Fake {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let f = Fake {
        zip: Arc::new(Mutex::new(Ok(good_build()))),
        url: Default::default(),
        base,
        list_hits: Default::default(),
        zip_hits: Default::default(),
    };
    let app = Router::new()
        .route("/cft.json", get(list))
        .route("/storage/*rest", get(storage))
        .route("/elsewhere/*rest", get(storage))
        .with_state(f.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    f
}

/// Everything in the cache folder (names only), to show nothing half-done is left behind.
fn left_in(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn installed_at(home: &Path) -> PathBuf {
    cache_root("linux", home).join(format!("{VERSION}-linux64")).join(binary_in("linux64"))
}

#[tokio::test]
async fn the_first_need_downloads_the_build_once_and_it_is_found_after() {
    let f = fake().await;
    let home = tempfile::tempdir().unwrap();
    let setup = Arc::new(f.setup(home.path()));
    assert!(setup.available());
    assert_eq!(setup.view().state, SetupState::Idle);
    assert_eq!(setup.installed(), None);

    // two bots needing a browser at once share one download
    let (a, b) = tokio::join!(setup.ensure(), setup.ensure());
    let want = installed_at(home.path());
    assert_eq!(a.unwrap(), want);
    assert_eq!(b.unwrap(), want);
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 1);
    assert_eq!(f.list_hits.load(Ordering::SeqCst), 1);

    let v = setup.view();
    assert_eq!(v.state, SetupState::Ready);
    assert_eq!(v.error, None);
    assert_eq!(v.version.as_deref(), Some(VERSION));
    assert_eq!(left_in(&cache_root("linux", home.path())), vec![format!("{VERSION}-linux64")]);
    let out = std::process::Command::new(&want).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains(VERSION));

    // the next need, and a restarted kwnt, use it without downloading again
    assert_eq!(setup.ensure().await.unwrap(), want);
    let after_restart = f.setup(home.path());
    assert_eq!(after_restart.view().state, SetupState::Ready);
    assert_eq!(after_restart.ensure().await.unwrap(), want);
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 1);
    assert_eq!(f.list_hits.load(Ordering::SeqCst), 1);

    // local mode finds it like any other Chrome, after an installed one
    assert_eq!(chrome_download::downloaded_chrome("linux", home.path()), Some(want.clone()));
    assert_eq!(find_chrome(None, "linux", Some(home.path()), Some("")), Some(want.clone()));
    let bin = tempfile::tempdir().unwrap();
    // never run here, so a plain executable file is enough
    std::fs::write(bin.path().join("chromium"), "#!/bin/sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.path().join("chromium"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(
        find_chrome(None, "linux", Some(home.path()), Some(&bin.path().to_string_lossy())),
        Some(bin.path().join("chromium"))
    );
}

#[tokio::test]
async fn a_failed_download_shows_the_line_and_try_again_fetches_it() {
    let f = fake().await;
    f.serve(Err(503));
    let home = tempfile::tempdir().unwrap();
    let setup = Arc::new(f.setup(home.path()));

    let err = setup.ensure().await.unwrap_err();
    assert!(err.starts_with(FAILED_LINE), "{err}");
    let v = setup.view();
    assert_eq!(v.state, SetupState::Failed);
    assert_eq!(
        v.error.as_deref(),
        Some("The bots' browser couldn't be set up. Bots can still answer; web pages won't open.")
    );
    assert!(v.detail.as_deref().unwrap().contains("503"), "{v:?}");
    assert_eq!(left_in(&cache_root("linux", home.path())), Vec::<String>::new());

    // the next bot that needs it gets the failure at once, without another download
    assert!(setup.ensure().await.unwrap_err().starts_with(FAILED_LINE));
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 1);

    // "Try again"
    f.serve(Ok(good_build()));
    setup.retry();
    for _ in 0..200 {
        if setup.view().state == SetupState::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let v = setup.view();
    assert_eq!(v.state, SetupState::Ready, "{v:?}");
    assert_eq!((v.error, v.detail), (None, None));
    assert_eq!(setup.installed(), Some(installed_at(home.path())));
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_broken_download_is_never_installed() {
    let f = fake().await;
    let no_browser = {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file("chrome-linux64/README", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hi").unwrap();
        zip.finish().unwrap();
        out.into_inner()
    };
    let cases = [
        (b"<html>not a zip</html>".to_vec(), "isn't a readable zip"),
        (no_browser, "has no browser inside"),
        (build("#!/bin/sh\necho 'error while loading shared libraries' >&2\nexit 127\n"), "doesn't start here"),
        (build("#!/bin/sh\necho hello\n"), "doesn't start here"),
    ];
    for (zip, why) in cases {
        f.serve(Ok(zip));
        let home = tempfile::tempdir().unwrap();
        let setup = f.setup(home.path());
        let err = setup.ensure().await.unwrap_err();
        assert!(err.starts_with(FAILED_LINE) && err.contains(why), "{why}: {err}");
        assert!(setup.view().detail.unwrap().contains(why));
        assert_eq!(setup.installed(), None, "{why}");
        assert_eq!(left_in(&cache_root("linux", home.path())), Vec::<String>::new(), "{why}");
        assert_eq!(find_chrome(None, "linux", Some(home.path()), Some("")), None, "{why}");
    }
}

#[tokio::test]
async fn a_download_outside_googles_storage_or_over_the_size_cap_is_refused() {
    let f = fake().await;
    *f.url.lock().unwrap() = Some(format!("{}/elsewhere/{VERSION}/linux64/chrome-linux64.zip", f.base));
    let home = tempfile::tempdir().unwrap();
    let err = f.setup(home.path()).ensure().await.unwrap_err();
    assert!(err.contains("somewhere other than Google's storage"), "{err}");
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 0);

    *f.url.lock().unwrap() = Some(format!("{}{VERSION}/../../elsewhere/linux64/chrome-linux64.zip", f.storage()));
    let err = f.setup(home.path()).ensure().await.unwrap_err();
    assert!(err.contains("somewhere other than Google's storage"), "{err}");
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 0);

    *f.url.lock().unwrap() = None;
    let err = f.setup(home.path()).max_bytes(1000).ensure().await.unwrap_err();
    assert!(err.contains("larger than"), "{err}");
    assert_eq!(left_in(&cache_root("linux", home.path())), Vec::<String>::new());
}

#[tokio::test]
async fn no_build_for_this_computer_downloads_nothing_and_shows_no_line() {
    let f = fake().await;
    let home = tempfile::tempdir().unwrap();
    // Linux on ARM, say: Google has no build
    let setup = Arc::new(ChromeSetup::new(cache_root("linux", home.path()), None).source(&f.json(), &f.storage()));
    assert!(!setup.available());
    assert!(setup.ensure().await.is_err());
    setup.retry();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let v = setup.view();
    assert_eq!(v.state, SetupState::Unavailable);
    assert_eq!(v.error, None);
    assert_eq!(f.list_hits.load(Ordering::SeqCst), 0);
    assert_eq!(f.zip_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn settings_show_the_setup_and_try_again_restarts_it() {
    let f = fake().await;
    f.serve(Err(500));
    let home = tempfile::tempdir().unwrap();
    let setup = f.setup(home.path());
    let data = tempfile::tempdir().unwrap();
    let env = common::setup_with(|state| {
        state.browser_service = Arc::new(
            kwnt::sandbox::service::BrowserService::new(data.path().to_path_buf(), None).with_chrome_setup(setup),
        );
    })
    .await;
    let (s, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["browser_setup"]["state"], "idle", "{v}");
    assert!(v["browser_setup"]["error"].is_null());

    // a bot needed the browser and the download failed
    assert!(env.state.browser_service.chrome_setup().ensure().await.is_err());
    let (_, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
    assert_eq!(v["browser_setup"]["state"], "failed");
    assert_eq!(v["browser_setup"]["error"], FAILED_LINE);

    // only the person can press Try again
    assert_eq!(common::call_remote(&env.app, "POST", "/api/settings/computer/browser").await, StatusCode::UNAUTHORIZED);

    f.serve(Ok(good_build()));
    let (s, v) = common::call(&env.app, "POST", "/api/settings/computer/browser", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(["downloading", "ready"].contains(&v["browser_setup"]["state"].as_str().unwrap()), "{v}");
    common::wait_for("the private browser", || async {
        let (_, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
        v["browser_setup"]["state"] == "ready"
    })
    .await;
    let (_, v) = common::call(&env.app, "GET", "/api/settings/computer", None).await;
    assert_eq!(v["browser_setup"]["version"], VERSION);
    assert!(v["browser_setup"]["error"].is_null());
}
