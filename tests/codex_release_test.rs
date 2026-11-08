//! Codex without Node (0.7.6, O4): `install_codex_release` against a local fake of Codex's
//! GitHub releases. Nothing here reaches the internet.
#![cfg(unix)]

#[path = "common/exe.rs"]
mod exe;

use axum::{
    extract::{Path as UrlPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use kwnt::setup::{install_codex_release, CodexInstall, CODEX_SUMS};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};

const TARGET: &str = "aarch64-unknown-linux-musl";

#[derive(Clone, Default)]
struct Release {
    tag: String,
    sums: String,
    archive: Vec<u8>,
    hits: Arc<Mutex<Vec<String>>>,
}

async fn latest(State(r): State<Release>) -> Response {
    r.hits.lock().unwrap().push("latest".into());
    (StatusCode::FOUND, [(header::LOCATION, format!("/releases/tag/{}", r.tag))]).into_response()
}

async fn tag_page(UrlPath(_tag): UrlPath<String>) -> &'static str {
    "<html>release</html>"
}

async fn download(State(r): State<Release>, UrlPath((tag, name)): UrlPath<(String, String)>) -> Response {
    r.hits.lock().unwrap().push(format!("{tag}/{name}"));
    if tag != r.tag {
        return StatusCode::NOT_FOUND.into_response();
    }
    if name == CODEX_SUMS {
        r.sums.clone().into_response()
    } else if name.starts_with("codex-package-") && name.ends_with(".tar.gz") {
        r.archive.clone().into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn serve(r: Release) -> String {
    let app = Router::new()
        .route("/releases/latest", get(latest))
        .route("/releases/tag/:tag", get(tag_page))
        .route("/releases/download/:tag/:name", get(download))
        .with_state(r);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/releases")
}

/// A codex package as the release ships it: bin/codex, its helper, and rg.
fn package(dir: &Path, version: &str) -> Vec<u8> {
    let pkg = dir.join(format!("pkg-{version}"));
    std::fs::create_dir_all(pkg.join("bin")).unwrap();
    std::fs::create_dir_all(pkg.join("codex-path")).unwrap();
    exe::write_executable(pkg.join("bin/codex"), format!("#!/bin/sh\necho \"codex-cli {version}\"\n"));
    exe::write_executable(pkg.join("bin/codex-code-mode-host"), "#!/bin/sh\n");
    exe::write_executable(pkg.join("codex-path/rg"), "#!/bin/sh\n");
    std::fs::write(pkg.join("codex-package.json"), "{}").unwrap();
    let out = dir.join(format!("pkg-{version}.tar.gz"));
    let ok = std::process::Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(&pkg).arg(".").status().unwrap();
    assert!(ok.success());
    std::fs::read(out).unwrap()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn release(tmp: &Path, version: &str) -> Release {
    let archive = package(tmp, version);
    let sums = format!(
        "{}  codex-app-server-package-{TARGET}.tar.gz\n{}  codex-package-{TARGET}.tar.gz\n{}  codex-package-x86_64-apple-darwin.tar.gz\n",
        sha(b"other"),
        sha(&archive),
        sha(b"mac")
    );
    Release { tag: format!("rust-v{version}"), sums, archive, ..Default::default() }
}

struct Run {
    at: CodexInstall,
    home: PathBuf,
    lines: Arc<Mutex<Vec<String>>>,
    _tmp: TempDir,
}

fn run_in(tmp: TempDir, url: &str) -> Run {
    let home = tmp.path().join("home");
    let at = CodexInstall::new(&home, None, TARGET).releases(url);
    Run { at, home, lines: Default::default(), _tmp: tmp }
}

async fn install(run: &Run) -> Result<PathBuf, String> {
    let lines = run.lines.clone();
    let log = move |l: String| lines.lock().unwrap().push(l);
    install_codex_release(&run.at, &log).await
}

fn version_of(cmd: &Path) -> String {
    let out = std::process::Command::new(cmd).arg("--version").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn installs_the_checked_build_into_local_bin_without_node() {
    let tmp = tempdir().unwrap();
    let rel = release(tmp.path(), "9.9.9");
    let hits = rel.hits.clone();
    let url = serve(rel).await;
    let run = run_in(tmp, &url);

    let cmd = install(&run).await.expect("installs");
    assert_eq!(cmd, run.home.join(".local/bin/codex"));
    assert_eq!(version_of(&cmd), "codex-cli 9.9.9");
    // Codex's own installer layout: releases/<version>-<target>, current, ~/.local/bin/codex
    let root = run.home.join(".codex/packages/standalone");
    let release = root.join(format!("releases/9.9.9-{TARGET}"));
    assert!(release.join("bin/codex").is_file());
    assert!(release.join("codex-path/rg").is_file());
    assert_eq!(std::fs::read_link(root.join("current")).unwrap(), release);
    assert_eq!(std::fs::read_link(&cmd).unwrap(), root.join("current/bin/codex"));
    // Linux: no helper link in ~/.local/bin (Codex's installer links it on a Mac only)
    assert!(!run.home.join(".local/bin/codex-code-mode-host").exists());
    // Nothing left behind: no archive, no staging folder
    let leftovers: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(leftovers.len(), 2, "{leftovers:?}");
    assert_eq!(std::fs::read_dir(root.join("releases")).unwrap().count(), 1);

    let lines = run.lines.lock().unwrap().join("\n");
    assert!(lines.contains("Codex 9.9.9"), "{lines}");
    assert!(lines.contains("SHA-256 matches"), "{lines}");
    assert!(lines.contains("codex-cli 9.9.9"), "{lines}");
    assert_eq!(
        *hits.lock().unwrap(),
        vec!["latest".to_string(), format!("rust-v9.9.9/{CODEX_SUMS}"), format!("rust-v9.9.9/codex-package-{TARGET}.tar.gz")]
    );
}

#[tokio::test]
async fn a_download_that_doesnt_match_the_release_sha256_is_never_installed() {
    let tmp = tempdir().unwrap();
    let mut rel = release(tmp.path(), "9.9.9");
    // Same size, one byte changed in transit
    let last = rel.archive.len() - 1;
    rel.archive[last] ^= 0xff;
    let url = serve(rel).await;
    let run = run_in(tmp, &url);

    let err = install(&run).await.unwrap_err();
    assert!(err.contains("doesn't match the SHA-256"), "{err}");
    assert!(!run.home.join(".local/bin/codex").exists());
    let root = run.home.join(".codex/packages/standalone");
    assert!(!root.join("releases").exists(), "unpacked an unchecked archive");
    assert!(!root.join("current").exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "the download was left behind");
}

#[tokio::test]
async fn a_release_without_a_checksum_for_this_computer_is_refused() {
    let tmp = tempdir().unwrap();
    let mut rel = release(tmp.path(), "9.9.9");
    rel.sums = rel.sums.lines().filter(|l| !l.ends_with(&format!("codex-package-{TARGET}.tar.gz"))).collect::<Vec<_>>().join("\n");
    let hits = rel.hits.clone();
    let url = serve(rel).await;
    let run = run_in(tmp, &url);
    let err = install(&run).await.unwrap_err();
    assert!(err.contains("has no SHA-256 for"), "{err}");
    assert!(!hits.lock().unwrap().iter().any(|h| h.ends_with(".tar.gz")), "downloaded without a checksum");
    assert!(!run.home.join(".local/bin/codex").exists());
}

#[tokio::test]
async fn a_newer_release_replaces_the_link_and_keeps_the_layout() {
    let tmp = tempdir().unwrap();
    let url = serve(release(tmp.path(), "9.9.9")).await;
    let run = run_in(tmp, &url);
    install(&run).await.unwrap();

    let newer = serve(release(run._tmp.path(), "9.10.0")).await;
    let at = run.at.clone().releases(&newer);
    let log = |_: String| {};
    let cmd = install_codex_release(&at, &log).await.unwrap();
    assert_eq!(version_of(&cmd), "codex-cli 9.10.0");
    let root = run.home.join(".codex/packages/standalone");
    assert_eq!(std::fs::read_link(root.join("current")).unwrap(), root.join(format!("releases/9.10.0-{TARGET}")));
    // Running it again is harmless
    install_codex_release(&at, &log).await.unwrap();
    assert_eq!(version_of(&cmd), "codex-cli 9.10.0");
}

#[tokio::test]
async fn a_mac_build_also_links_the_code_mode_helper_and_codex_home_is_respected() {
    let tmp = tempdir().unwrap();
    let mut rel = release(tmp.path(), "9.9.9");
    rel.sums = rel.sums.replace(&format!("codex-package-{TARGET}"), "codex-package-aarch64-apple-darwin");
    let hits = rel.hits.clone();
    let url = serve(rel).await;
    let home = tmp.path().join("home");
    let at = CodexInstall::new(&home, Some(tmp.path().join("codex-home")), "aarch64-apple-darwin").releases(&url);
    let cmd = install_codex_release(&at, &|_| {}).await.unwrap();
    assert!(hits.lock().unwrap().iter().any(|h| h.ends_with("codex-package-aarch64-apple-darwin.tar.gz")));
    let current = tmp.path().join("codex-home/packages/standalone/current");
    assert_eq!(std::fs::read_link(home.join(".local/bin/codex-code-mode-host")).unwrap(), current.join("bin/codex-code-mode-host"));
    assert_eq!(std::fs::read_link(&cmd).unwrap(), current.join("bin/codex"));
    assert!(!home.join(".codex").exists());
}

#[tokio::test]
async fn a_latest_that_isnt_a_codex_tag_is_refused() {
    let tmp = tempdir().unwrap();
    let mut rel = release(tmp.path(), "9.9.9");
    rel.tag = "nightly".into();
    let url = serve(rel).await;
    let run = run_in(tmp, &url);
    let err = install(&run).await.unwrap_err();
    assert!(err.contains("couldn't tell the latest Codex release"), "{err}");
}
