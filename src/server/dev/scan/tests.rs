//! Owner tests for qa/plan.md §3.5 (J-S1–J-S4, J-P). Every network call goes to a mock on
//! 127.0.0.1:9475–9478; fake keys are joined at runtime (see `testkit`). Tests that need the real
//! gitleaks binary run only when `KWANT_TEST_GITLEAKS` names one (never in CI, which can't fetch).

use super::gitleaks::{self, Source, Target, PINS};
use super::packages::{self, Eco, Registries};
use super::testkit::*;
use super::*;
use crate::zombie::github::GhFile;
use chrono::{Duration as Days, TimeZone, Utc};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

fn gh(path: &str, status: &str, patch: Option<&str>) -> GhFile {
    let adds = patch.map_or(0, |p| p.lines().filter(|l| l.starts_with('+')).count() as u64);
    let dels = patch.map_or(0, |p| p.lines().filter(|l| l.starts_with('-')).count() as u64);
    GhFile { filename: path.into(), status: status.into(), additions: adds, deletions: dels, patch: patch.map(Into::into), previous_filename: None }
}

fn diff(path: &str, status: &str, patch: Option<&str>) -> FileDiff {
    let g = gh(path, status, patch);
    FileDiff { path: g.filename, status: g.status, additions: g.additions, deletions: g.deletions, patch: g.patch, previous_path: None }
}

/// A Config whose gitleaks comes from the mock, cached in `dir`, and whose registries all fail
/// fast (no package test should reach them through this).
fn cfg(dir: &std::path::Path, base: &str, archive_sha: &str, bin_sha: &str) -> Config {
    Config {
        tools_dir: dir.to_path_buf(),
        gitleaks: Some(Source {
            url: format!("{base}/gl.tar.gz"),
            archive: "gl.tar.gz",
            archive_sha256: archive_sha.into(),
            binary_sha256: bin_sha.into(),
            fetch_timeout: Duration::from_secs(5),
        }),
        gitleaks_timeout: Duration::from_secs(20),
        registries: Registries::all_at("http://127.0.0.1:9476"),
    }
}

/// Serves a fake gitleaks release made from `extra` and returns the mock and a Config for it.
async fn fake(dir: &std::path::Path, extra: &str) -> (Mock, Config) {
    let (archive, a, b) = fake_release(&fake_gitleaks(extra));
    let mock = serve(MOCK_PORT, move |r| if r.path == "/gl.tar.gz" { Reply::bytes(200, archive.clone()) } else { Reply::bytes(404, vec![]) }).await;
    let c = cfg(dir, &mock.base, &a, &b);
    (mock, c)
}

fn secret(n: u32) -> String {
    format!("FAKE_SECRET_{n}")
}

// ---------------------------------------------------------------------------------------------
// J-S1: pinned version and checksum, refuse on mismatch, no network, failed fetch is not_run

#[test]
fn j_s1_version_and_checksums_are_pinned_in_source_for_every_shipped_platform() {
    assert_eq!(gitleaks::VERSION, "8.30.1");
    let want = [("macos", "aarch64", "darwin_arm64"), ("macos", "x86_64", "darwin_x64"), ("linux", "aarch64", "linux_arm64"), ("linux", "x86_64", "linux_x64")];
    for (os, arch, tag) in want {
        let s = Source::for_platform(os, arch).unwrap_or_else(|| panic!("{os} {arch} has no pin"));
        assert_eq!(s.url, format!("https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_{tag}.tar.gz"));
        for sha in [&s.archive_sha256, &s.binary_sha256] {
            assert_eq!(sha.len(), 64);
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{sha}");
        }
    }
    assert_eq!(PINS.len(), 4);
    let shas: BTreeSet<&str> = PINS.iter().flat_map(|p| [p.archive_sha256, p.binary_sha256]).collect();
    assert_eq!(shas.len(), 8, "every checksum is different");
    assert!(Source::for_platform("windows", "x86_64").is_none());
    assert!(Source::for_platform("linux", "riscv64").is_none());
}

#[tokio::test]
async fn j_s1_unpinned_platform_reads_not_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg(dir.path(), "http://127.0.0.1:9475", "00", "00");
    c.gitleaks = None;
    let s = scan(&c, &Input { files: vec![diff("a.rs", "added", Some(&added_patch(&["let x = 1;"])))], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
    assert!(s.security.secrets.reason.as_deref().unwrap().contains("no pinned build"));
}

#[tokio::test]
async fn j_s1_fetched_once_checked_against_both_checksums_and_cached() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (mock, c) = fake(dir.path(), "").await;
    let src = c.gitleaks.clone().unwrap();
    let bin = gitleaks::ensure(&c.tools_dir, &src).await.unwrap();
    assert_eq!(gitleaks::sha256_hex(&std::fs::read(&bin).unwrap()), src.binary_sha256);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o755);
    }
    gitleaks::ensure(&c.tools_dir, &src).await.unwrap();
    assert_eq!(mock.requests().len(), 1, "the second use reads the cache");
    // A cached binary that changed on disk is never run: it is fetched again
    std::fs::write(&bin, b"#!/bin/sh\nexit 0\n").unwrap();
    let again = gitleaks::ensure(&c.tools_dir, &src).await.unwrap();
    assert_eq!(gitleaks::sha256_hex(&std::fs::read(&again).unwrap()), src.binary_sha256);
    assert_eq!(mock.requests().len(), 2);
}

#[tokio::test]
async fn j_s1_a_wrong_checksum_refuses_to_run_and_reads_not_run() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let (_mock, mut c) = fake(dir.path(), &format!("touch '{}'", marker.display())).await;
    let files = vec![diff("a.rs", "added", Some(&added_patch(&[&secret(1)])))];
    // The archive doesn't match
    let good = c.gitleaks.clone().unwrap();
    c.gitleaks.as_mut().unwrap().archive_sha256 = "0".repeat(64);
    let s = scan(&c, &Input { files: files.clone(), ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
    assert!(s.security.secrets.reason.as_deref().unwrap().contains("didn't match its pinned checksum"), "{:?}", s.security.secrets);
    // The archive matches but the binary inside doesn't
    c.gitleaks = Some(Source { binary_sha256: "f".repeat(64), ..good });
    let s = scan(&c, &Input { files, ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
    assert!(s.security.secrets.reason.as_deref().unwrap().contains("binary didn't match"));
    assert!(!marker.exists(), "a binary that failed its checksum never runs");
    assert!(std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).all(|e| !e.path().join("gitleaks").exists()));
}

#[tokio::test]
async fn j_s1_a_failed_or_timed_out_download_reads_not_run() {
    let _l = port_lock().await;
    let files = vec![diff("a.rs", "added", Some(&added_patch(&[&secret(1)])))];
    for (status, slow, want) in [(500, false, "HTTP 500"), (404, false, "HTTP 404"), (429, false, "HTTP 429"), (200, true, "timed out")] {
        let dir = tempfile::tempdir().unwrap();
        let mock = serve(MOCK_PORT, move |_| {
            let r = Reply::bytes(status, b"nope".to_vec());
            if slow {
                r.slow(Duration::from_secs(3))
            } else {
                r
            }
        })
        .await;
        let mut c = cfg(dir.path(), &mock.base, &"0".repeat(64), &"0".repeat(64));
        c.gitleaks.as_mut().unwrap().fetch_timeout = Duration::from_millis(500);
        let s = scan(&c, &Input { files: files.clone(), ..Input::default() }).await;
        assert_eq!(s.security.secrets.state, State::NotRun, "{status}");
        assert!(s.security.secrets.reason.as_deref().unwrap().contains(want), "{status}: {:?}", s.security.secrets.reason);
        drop(mock);
    }
    // Nothing listening at all
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), "http://127.0.0.1:9478", &"0".repeat(64), &"0".repeat(64));
    let s = scan(&c, &Input { files, ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
}

#[tokio::test]
async fn j_s1_gitleaks_runs_with_no_network_and_a_clean_environment() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("env.txt");
    let leak = serve(LEAK_PORT, |_| Reply::bytes(200, b"leaked".to_vec())).await;
    let extra = format!(
        "env > '{f}'\ncurl -s -m 3 http://127.0.0.1:{LEAK_PORT}/from-gitleaks >/dev/null 2>&1\necho \"curl_exit=$?\" >> '{f}'\ncurl -s -m 3 --noproxy '' http://127.0.0.1:{LEAK_PORT}/from-gitleaks-2 >/dev/null 2>&1\necho \"curl_exit=$?\" >> '{f}'",
        f = env_file.display()
    );
    let (_mock, c) = fake(dir.path(), &extra).await;
    let s = scan(&c, &Input { files: vec![diff("a.rs", "added", Some(&added_patch(&["let a = 1;"])))], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::Pass, "{:?}", s.security.secrets);
    assert!(leak.requests().is_empty(), "gitleaks reached the network: {:?}", leak.requests());
    let env = std::fs::read_to_string(&env_file).unwrap();
    assert!(env.contains("HTTPS_PROXY=http://127.0.0.1:9") && env.contains("http_proxy=http://127.0.0.1:9"), "{env}");
    // Both attempts ran (curl exists) and failed (no answer came back)
    let exits: Vec<&str> = env.lines().filter_map(|l| l.strip_prefix("curl_exit=")).collect();
    assert_eq!(exits.len(), 2, "{env}");
    assert!(exits.iter().all(|c| *c != "0" && *c != "127"), "curl exits: {exits:?}");
    // Nothing from kwant's own environment reaches it
    for (k, _) in std::env::vars() {
        if !["PATH", "HOME", "TMPDIR", "PWD", "SHLVL", "_", "OLDPWD"].contains(&k.as_str()) && !k.to_ascii_lowercase().ends_with("proxy") {
            assert!(!env.lines().any(|l| l.starts_with(&format!("{k}="))), "{k} was passed to gitleaks");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// J-S2: added lines only; the value hidden everywhere

#[tokio::test]
async fn j_s2_only_added_lines_are_scanned_at_their_new_line_numbers() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_mock, c) = fake(dir.path(), "").await;
    // A removed secret and a secret in an unchanged context line don't hit
    let removed = format!("@@ -10,3 +10,3 @@ fn main() {{\n let a = 1;\n-let k = \"{}\";\n+let k = std::env::var(\"K\");\n  {}", secret(1), secret(2));
    let s = scan(&c, &Input { files: vec![diff("src/main.rs", "modified", Some(&removed))], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::Pass, "{:?}", s.security);
    assert_eq!(s.security.secrets.count, 0);
    // An added one hits at its new-side line number
    let added = format!("@@ -10,3 +10,4 @@ fn main() {{\n let a = 1;\n-let b = 2;\n+let b = 3;\n+let k = \"{}\";\n let c = 4;", secret(3));
    let s = scan(&c, &Input { files: vec![diff("src/main.rs", "modified", Some(&added))], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::Hit);
    assert_eq!(s.security.secrets.count, 1);
    let f = &s.security.findings[0];
    assert_eq!((f.id.as_str(), f.file.as_str(), f.line, f.source.as_str(), f.severity.as_str()), ("s1", "src/main.rs", Some(12), "gitleaks", "critical"));
    assert_eq!(s.hide["src/main.rs"], vec![(12, 12)]);
}

#[tokio::test]
async fn j_s2_the_value_is_hidden_from_facts_card_findings_and_fix_it_text() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_mock, c) = fake(dir.path(), "").await;
    let value = secret(424242);
    let patch = format!("@@ -1,2 +1,4 @@\n use x;\n+// demo config for example.com\n+const KEY: &str = \"{value}\";\n fn f() {{}}");
    let mut files = vec![gh("src/config.rs", "modified", Some(&patch)), gh("README.md", "modified", Some("@@ -1 +1,2 @@\n # demo\n+More text."))];
    let sec = scan_and_hide(&c, &mut files, BTreeMap::new(), None).await;
    assert_eq!(sec.secrets.state, State::Hit);
    // The stored patch keeps its shape, with the line replaced
    let p = files[0].patch.as_deref().unwrap();
    assert!(p.contains(&format!("+{HIDDEN_LINE}")) && p.contains("+// demo config for example.com") && p.lines().count() == 5, "{p}");
    // Facts (what the model reads), the card's security and findings, and the Fix it task
    let (diffs, _) = crate::server::dev::poller::cut_diffs(files.clone());
    let facts = serde_json::to_string(&json!({"files": diffs, "security": sec})).unwrap();
    let card = json!({"security": sec.card_json(), "findings": sec.findings.iter().map(Finding::card_json).collect::<Vec<_>>()}).to_string();
    let space = crate::db::spaces::SpaceId::new("space-demo");
    let mut texts = vec![facts, card];
    for f in &sec.findings {
        let t = crate::server::dev::routes::fix_task(&space, "kwant-demo", "card-demo", &f.id, &f.card_json(), None);
        texts.push(format!("{} {}", t.title, t.acceptance));
    }
    for t in &texts {
        assert!(!t.contains(&value) && !t.contains("FAKE_SECRET"), "the value leaked: {t}");
    }
    assert!(texts[2].contains("src/config.rs:3"), "{}", texts[2]);
}

#[test]
fn j_s2_hide_lines_replaces_whole_added_lines_and_keeps_numbers() {
    let p = "@@ -1,3 +1,5 @@\n a\n+b\n+c\n-d\n+e\n f\n@@ -20,1 +22,2 @@\n g\n+h";
    let out = hide_lines(p, &[(3, 4), (23, 23)]);
    assert_eq!(out, format!("@@ -1,3 +1,5 @@\n a\n+b\n+{HIDDEN_LINE}\n-d\n+{HIDDEN_LINE}\n f\n@@ -20,1 +22,2 @@\n g\n+{HIDDEN_LINE}"));
    assert_eq!(gitleaks::added_lines(p).iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![2, 3, 4, 23]);
}

// Real gitleaks (local only): `KWANT_TEST_GITLEAKS=/path/to/gitleaks-8.30.1 cargo test j_s -- --ignored`

fn real_bin() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var("KWANT_TEST_GITLEAKS").ok()?);
    p.is_file().then_some(p)
}

fn target(path: &str, lines: &[String]) -> Target {
    Target { path: path.into(), lines: lines.iter().enumerate().map(|(i, l)| (i as u64 + 1, l.clone())).collect() }
}

#[tokio::test]
#[ignore = "needs KWANT_TEST_GITLEAKS (the real binary); CI has no network to fetch it"]
async fn j_s2_real_gitleaks_finds_runtime_built_aws_github_stripe_and_private_keys() {
    let Some(bin) = real_bin() else { return };
    let keys = [
        ("aws.py", format!("aws_access_key_id = \"{}\"", fake_aws_key())),
        ("gh.js", format!("const token = \"{}\";", fake_github_token())),
        ("pay.rb", format!("Stripe.api_key = \"{}\"", fake_stripe_key())),
        ("id_demo", fake_private_key()),
    ];
    let targets: Vec<Target> = keys.iter().map(|(p, v)| target(p, &v.lines().map(String::from).collect::<Vec<_>>())).collect();
    let hits = gitleaks::run(&bin, &targets, Duration::from_secs(60)).await.unwrap();
    for (p, _) in &keys {
        assert!(hits.iter().any(|h| h.file == *p), "no hit in {p}: {hits:?}");
    }
    let debug = format!("{hits:?}");
    for (_, v) in &keys {
        for line in v.lines().filter(|l| l.len() > 20) {
            assert!(!debug.contains(line), "a value came back from the report");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// J-S3: false positives

#[tokio::test]
#[ignore = "needs KWANT_TEST_GITLEAKS (the real binary); CI has no network to fetch it"]
async fn j_s3_real_gitleaks_ignores_checksums_integrity_uuids_shas_and_base64_images() {
    let Some(bin) = real_bin() else { return };
    let sha = |n: u64| (0..4).map(|i| format!("{:016x}", n.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(i * 13))).collect::<String>();
    let png: String = {
        use base64::Engine;
        let bytes: Vec<u8> = (0..600u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    };
    let corpus = vec![
        target("Cargo.lock", &["[[package]]".into(), "name = \"demo\"".into(), "version = \"1.0.0\"".into(), format!("checksum = \"{}\"", sha(1))]),
        target(
            "package-lock.json",
            &[format!("      \"integrity\": \"sha512-{}\",", &png[..86]), "      \"resolved\": \"https://registry.npmjs.org/demo/-/demo-1.0.0.tgz\",".into()],
        ),
        target("src/ids.rs", &["const ID: &str = \"3f2b8c1e-9a4d-4e7b-8c2f-1d6e5a9b0c3f\";".into(), "let id = Uuid::parse_str(\"a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d\");".into()]),
        target("docs/CHANGELOG.md", &[format!("- Fixed in commit {} (see {}).", &sha(2)[..40], &sha(3)[..7]), format!("Release {}", &sha(4)[..40])]),
        target("web/logo.html", &[format!("<img src=\"data:image/png;base64,{png}\" alt=\"demo\">")]),
    ];
    let hits = gitleaks::run(&bin, &corpus, Duration::from_secs(60)).await.unwrap();
    assert!(hits.is_empty(), "false positives: {hits:?}");
}

// ---------------------------------------------------------------------------------------------
// J-S4: pass only over the whole diff

#[tokio::test]
async fn j_s4_files_cut_from_the_models_60k_are_still_scanned() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_mock, c) = fake(dir.path(), "").await;
    let big: Vec<String> = (0..2_000).map(|i| format!("let filler_{i} = {i}; // padding for the 60k cut")).collect();
    let big_patch = added_patch(&big.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(big_patch.len() > super::super::facts::DIFF_CHARS);
    let late = added_patch(&["fn demo() {}", &format!("let k = \"{}\";", secret(7))]);
    let mut files = vec![gh("src/a_small.rs", "added", Some("@@ -0,0 +1 @@\n+fn a() {}")), gh("src/b_big.rs", "added", Some(&big_patch)), gh("src/c_late.rs", "added", Some(&late))];
    let sec = scan_and_hide(&c, &mut files, BTreeMap::new(), None).await;
    let (diffs, cut) = crate::server::dev::poller::cut_diffs(files);
    assert!(cut.is_some() && diffs[1].patch.is_none(), "the big file is cut for the model");
    assert_eq!(sec.secrets.state, State::Hit, "{sec:?}");
    assert_eq!((sec.findings[0].file.as_str(), sec.findings[0].line), ("src/c_late.rs", Some(2)));
}

#[tokio::test]
async fn j_s4_a_timeout_a_crash_or_a_bad_report_is_never_a_pass() {
    let _l = port_lock().await;
    let files = vec![diff("a.rs", "added", Some(&added_patch(&["let a = 1;"])))];
    for (extra, want) in [("sleep 5", "didn't finish"), ("exit 3", "stopped with an error"), ("echo 'not json' > \"$out\"; exit 0", "couldn't read")] {
        let dir = tempfile::tempdir().unwrap();
        let (mock, mut c) = fake(dir.path(), extra).await;
        c.gitleaks_timeout = Duration::from_secs(1);
        let s = scan(&c, &Input { files: files.clone(), ..Input::default() }).await;
        assert_eq!(s.security.secrets.state, State::NotRun, "{extra}");
        assert!(s.security.secrets.reason.as_deref().unwrap().contains(want), "{extra}: {:?}", s.security.secrets.reason);
        drop(mock);
    }
}

#[tokio::test]
async fn j_s4_unread_files_and_a_capped_file_list_read_not_run_even_with_a_hit_elsewhere() {
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_mock, c) = fake(dir.path(), "").await;
    // GitHub sent no patch for a large text file, and its text couldn't be fetched
    let mut big = diff("src/generated.rs", "modified", None);
    big.additions = 900;
    let ok = diff("src/a.rs", "added", Some(&added_patch(&["let a = 1;"])));
    let s = scan(&c, &Input { files: vec![ok.clone(), big.clone()], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
    assert!(s.security.secrets.reason.as_deref().unwrap().contains("src/generated.rs"));
    // Its head text was fetched: it is scanned whole
    let contents = BTreeMap::from([("src/generated.rs".to_string(), format!("a\nb\nlet k = \"{}\";\n", secret(9)))]);
    let s = scan(&c, &Input { files: vec![ok.clone(), big.clone()], contents, ..Input::default() }).await;
    assert_eq!((s.security.secrets.state, s.security.findings[0].line), (State::Hit, Some(3)));
    // Lock files, binaries and removed files need no text
    let mut lock = diff("Cargo.lock", "modified", None);
    lock.additions = 40;
    let mut png = diff("web/logo.png", "added", None);
    png.additions = 1;
    let s = scan(&c, &Input { files: vec![ok.clone(), lock, png, diff("old.rs", "removed", None)], ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::Pass, "{:?}", s.security.secrets);
    // GitHub's file list stopped at its cap
    let s = scan(&c, &Input { files: vec![ok], files_capped: Some("the PR has more than 300 files".into()), ..Input::default() }).await;
    assert_eq!(s.security.secrets.state, State::NotRun);
}

#[test]
fn j_s4_paths_from_the_pr_stay_inside_the_scan_tree() {
    for (p, want) in [
        ("../../etc/passwd", "0/etc/passwd"),
        ("a/./b/../c", "0/a/b/c"),
        ("/abs/x", "0/abs/x"),
        (".gitleaks.toml", "0/_.gitleaks.toml"),
        ("sub/.gitleaksignore", "0/sub/_.gitleaksignore"),
        ("..", "0/file"),
    ] {
        assert_eq!(gitleaks::safe_rel_for_test(0, p), std::path::PathBuf::from(want), "{p}");
    }
}

#[test]
fn j_s4_the_tar_reader_finds_only_the_gitleaks_file() {
    let t = tar(&[("README.md", 0o644, b"gitleaks"), ("gitleaks", 0o755, b"#!/bin/sh\n")]);
    assert_eq!(gitleaks::tar_entry(&t, "gitleaks").unwrap(), b"#!/bin/sh\n");
    assert!(gitleaks::tar_entry(&tar(&[("other", 0o755, b"x")]), "gitleaks").is_none());
    assert!(gitleaks::unpack(b"not gzip").is_err());
    assert_eq!(gitleaks::unpack(&gzip(&t)).unwrap(), b"#!/bin/sh\n");
}

// ---------------------------------------------------------------------------------------------
// J-P: the package check

fn manifest(path: &str, status: &str, before: &str, after: &str) -> (FileDiff, String) {
    // A patch that replaces the whole file, which is what base_text undoes
    let old: Vec<&str> = if before.is_empty() { vec![] } else { before.split('\n').collect() };
    let new: Vec<&str> = after.split('\n').collect();
    let mut p = format!("@@ -{},{} +1,{} @@", if old.is_empty() { 0 } else { 1 }, old.len(), new.len());
    for l in &old {
        p.push_str(&format!("\n-{l}"));
    }
    for l in &new {
        p.push_str(&format!("\n+{l}"));
    }
    (diff(path, status, Some(&p)), after.to_string())
}

fn names(files: &[(FileDiff, String)]) -> Vec<(Eco, String)> {
    let contents: BTreeMap<String, String> = files.iter().map(|(f, t)| (f.path.clone(), t.clone())).collect();
    let diffs: Vec<FileDiff> = files.iter().map(|(f, _)| f.clone()).collect();
    packages::added(&diffs, &contents).deps.into_iter().map(|d| (d.eco, d.name)).collect()
}

#[test]
fn j_p_dependency_forms_and_only_new_names() {
    let cargo_before = "[package]\nname = \"demo-app\"\n\n[dependencies]\nserde = \"1.0\"\n";
    let cargo_after = "[package]\nname = \"demo-app\"\n\n[dependencies]\nserde = \"1.1\"\nfresh-crate = \"0.1\"\nlocal = { path = \"../local\" }\nfrom-git = { git = \"https://example.com/demo.git\" }\nshared = { workspace = true }\nrenamed = { package = \"real-crate\", version = \"2\" }\n\n[dev-dependencies]\ndev-only = \"1\"\n\n[target.'cfg(unix)'.build-dependencies]\nbuild-only = \"1\"\n";
    let npm_after = r#"{"name": "demo-web", "dependencies": {"@demo-scope/widget": "^1.0.0", "left": "file:../left", "ws-pkg": "workspace:*", "gitdep": "github:demo/repo", "short": "demo/repo", "aliased": "npm:@real-scope/real@^2"}, "devDependencies": {"devtool": "~3"}}"#;
    let got = names(&[manifest("Cargo.toml", "modified", cargo_before, cargo_after), manifest("web/package.json", "added", "", npm_after)]);
    let want: Vec<(Eco, String)> = vec![
        (Eco::Crates, "build-only".into()),
        (Eco::Crates, "dev-only".into()),
        (Eco::Crates, "fresh-crate".into()),
        (Eco::Crates, "real-crate".into()),
        (Eco::Npm, "@demo-scope/widget".into()),
        (Eco::Npm, "@real-scope/real".into()),
        (Eco::Npm, "devtool".into()),
    ];
    let mut got_sorted = got.clone();
    got_sorted.sort();
    let mut want_sorted = want;
    want_sorted.sort();
    assert_eq!(got_sorted, want_sorted, "a version bump (serde) isn't new; path, git, workspace and URL deps aren't registry packages");
}

#[test]
fn j_p_python_manifests() {
    let req = manifest("requirements-dev.txt", "modified", "requests==2.31\n", "requests==2.32\nFlask_Login>=0.6  # auth\n-e ./local\ndemo @ https://example.com/demo.whl\nhttpx[http2]>=0.27; python_version>'3.8'\n");
    let py = manifest(
        "pyproject.toml",
        "added",
        "",
        "[project]\nname = \"demo-py\"\ndependencies = [\"rich>=13\", \"fresh-py-lib\"]\n[project.optional-dependencies]\nextra = [\"other-lib\"]\n[tool.poetry.dependencies]\npython = \"^3.11\"\nlocal = { path = \"../x\" }\n",
    );
    let mut got = names(&[req, py]);
    got.sort();
    assert_eq!(
        got,
        vec![(Eco::PyPi, "Flask_Login".to_string()), (Eco::PyPi, "fresh-py-lib".into()), (Eco::PyPi, "httpx".into()), (Eco::PyPi, "other-lib".into()), (Eco::PyPi, "rich".into())]
    );
    assert_eq!(Eco::PyPi.norm("Flask__Login.ext"), "flask-login-ext");
    assert_eq!(packages::segment(Eco::PyPi, "Flask_Login").unwrap(), "flask-login");
}

#[tokio::test]
async fn j_p_custom_registries_are_skipped_and_read_not_run_with_the_reason() {
    let cases = [
        manifest("Cargo.toml", "added", "", "[dependencies]\ninternal = { version = \"1\", registry = \"demo-corp\" }\n"),
        manifest("requirements.txt", "added", "", "--index-url https://pypi.example.com/simple\ninternal-py\n"),
        manifest("pyproject.toml", "added", "", "[tool.poetry.dependencies]\ninternal = { version = \"1\", source = \"corp\" }\n"),
        manifest("py2/pyproject.toml", "added", "", "[project]\ndependencies = [\"internal\"]\n[[tool.uv.index]]\nname = \"corp\"\nurl = \"https://pypi.example.com\"\ndefault = true\n"),
    ];
    for (f, text) in cases {
        let contents = BTreeMap::from([(f.path.clone(), text)]);
        let (tile, findings) = packages::run(&Registries::all_at("http://127.0.0.1:9478"), std::slice::from_ref(&f), &contents).await;
        assert_eq!(tile.state, State::NotRun, "{}", f.path);
        assert!(tile.reason.as_deref().unwrap().contains("custom"), "{}: {:?}", f.path, tile.reason);
        assert!(findings.is_empty());
    }
}

fn npm_doc(created: chrono::DateTime<Utc>) -> serde_json::Value {
    json!({"name": "x", "time": {"created": created.to_rfc3339(), "modified": created.to_rfc3339()}})
}

/// Serves npm, PyPI and crates answers from `table` (path → reply); anything else is 404.
async fn registry(table: Vec<(String, u16, serde_json::Value, u64)>) -> Mock {
    serve(MOCK_PORT, move |r| {
        match table.iter().find(|(p, ..)| *p == r.path) {
            Some((_, s, v, slow)) => Reply::json(*s, v.clone()).slow(Duration::from_millis(*slow)),
            None => Reply::json(404, json!({"error": "not found"})),
        }
    })
    .await
}

fn regs(base: &str, now: chrono::DateTime<Utc>) -> Registries {
    Registries { timeout: Duration::from_millis(400), budget: Duration::from_secs(10), now: Some(now), ..Registries::all_at(base) }
}

async fn check_one(eco: Eco, name: &str, r: &Registries) -> (Tile, Vec<Finding>) {
    let (path, text) = match eco {
        Eco::Npm => ("package.json", format!(r#"{{"dependencies": {{"{name}": "^1"}}}}"#)),
        Eco::PyPi => ("requirements.txt", format!("{name}\n")),
        Eco::Crates => ("Cargo.toml", format!("[dependencies]\n\"{name}\" = \"1\"\n")),
    };
    let (f, t) = manifest(path, "added", "", &text);
    packages::run(r, std::slice::from_ref(&f), &BTreeMap::from([(f.path.clone(), t)])).await
}

#[tokio::test]
async fn j_p_registry_answers_404_is_missing_and_5xx_429_or_timeout_is_not_run() {
    let _l = port_lock().await;
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap();
    let old = now - Days::days(400);
    let mock = registry(vec![
        ("/npm/demo-fine".into(), 200, npm_doc(old), 0),
        ("/npm-downloads/downloads/point/last-week/demo-fine".into(), 200, json!({"downloads": 5000}), 0),
        ("/npm/demo-broken".into(), 503, json!({}), 0),
        ("/npm/demo-limited".into(), 429, json!({}), 0),
        ("/npm/demo-slow".into(), 200, npm_doc(old), 2_000),
        ("/crates/api/v1/crates/demo-crate".into(), 500, json!({}), 0),
    ])
    .await;
    let r = regs(&mock.base, now);
    let (t, f) = check_one(Eco::Npm, "demo-fine", &r).await;
    assert_eq!((t.state, t.count, f.len()), (State::Pass, 0, 0), "{t:?}");
    let (t, f) = check_one(Eco::Npm, "demo-ghost", &r).await;
    assert_eq!((t.state, t.count, t.missing), (State::Hit, 1, 1));
    assert_eq!((f[0].severity.as_str(), f[0].source.as_str(), f[0].id.as_str()), ("critical", "packages", "p1"));
    assert!(f[0].title.contains("doesn't exist"), "{}", f[0].title);
    for (eco, name, why) in [(Eco::Npm, "demo-broken", "503"), (Eco::Npm, "demo-limited", "429"), (Eco::Npm, "demo-slow", "timed out"), (Eco::Crates, "demo-crate", "500")] {
        let (t, f) = check_one(eco, name, &r).await;
        assert_eq!(t.state, State::NotRun, "{name}");
        assert!(t.reason.as_deref().unwrap().contains(why), "{name}: {:?}", t.reason);
        assert!(f.is_empty());
    }
}

#[tokio::test]
async fn j_p_age_boundary_and_low_downloads() {
    let _l = port_lock().await;
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap();
    let mut table = Vec::new();
    for d in [29, 30, 31] {
        table.push((format!("/npm/demo-age-{d}"), 200, npm_doc(now - Days::days(d)), 0));
        table.push((format!("/npm-downloads/downloads/point/last-week/demo-age-{d}"), 200, json!({"downloads": 10_000}), 0));
    }
    for n in [99, 100] {
        table.push((format!("/npm/demo-dl-{n}"), 200, npm_doc(now - Days::days(900)), 0));
        table.push((format!("/npm-downloads/downloads/point/last-week/demo-dl-{n}"), 200, json!({"downloads": n}), 0));
    }
    table.push(("/crates/api/v1/crates/demo-few".into(), 200, json!({"crate": {"created_at": (now - Days::days(900)).to_rfc3339(), "recent_downloads": 999}}), 0));
    table.push((
        "/pypi/pypi/demo-young/json".into(),
        200,
        json!({"releases": {"0.1": [{"upload_time_iso_8601": (now - Days::days(3)).to_rfc3339()}], "0.0.1": []}}),
        0,
    ));
    table.push(("/pypistats/api/packages/demo-young/recent".into(), 200, json!({"data": {"last_week": 5000}}), 0));
    let mock = registry(table).await;
    let r = regs(&mock.base, now);
    let flagged = |t: &Tile| t.state == State::Hit;
    assert!(flagged(&check_one(Eco::Npm, "demo-age-29", &r).await.0), "29 days is new");
    assert!(!flagged(&check_one(Eco::Npm, "demo-age-30", &r).await.0), "30 days is not");
    assert!(!flagged(&check_one(Eco::Npm, "demo-age-31", &r).await.0));
    let (t, f) = check_one(Eco::Npm, "demo-dl-99", &r).await;
    assert!(flagged(&t) && t.missing == 0 && f[0].detail.contains("only 99 downloads"), "{f:?}");
    assert!(!flagged(&check_one(Eco::Npm, "demo-dl-100", &r).await.0));
    let (t, f) = check_one(Eco::Crates, "demo-few", &r).await;
    assert!(flagged(&t) && f[0].detail.contains("999"));
    let (t, f) = check_one(Eco::PyPi, "demo_young", &r).await;
    assert!(flagged(&t) && f[0].detail.contains("first published 3 days ago"), "{f:?}");
}

#[test]
fn j_p_one_letter_off_a_popular_name() {
    let none = BTreeSet::new();
    assert_eq!(packages::near_popular(Eco::Npm, "lodahs", &none), Some("lodash"), "a swap");
    assert_eq!(packages::near_popular(Eco::Npm, "expresss", &none), Some("express"), "an extra letter");
    assert_eq!(packages::near_popular(Eco::Npm, "@evil/axois", &none), Some("axios"), "inside a scope");
    assert_eq!(packages::near_popular(Eco::PyPi, "reqests", &none), Some("requests"), "a missing letter");
    assert_eq!(packages::near_popular(Eco::Crates, "tokoi", &none), Some("tokio"));
    assert_eq!(packages::near_popular(Eco::Crates, "serde_jsom", &none), Some("serde-json"), "crates compare - and _ as one");
    // Legitimate pairs on the list, short names, unrelated names and the repo's own names
    for (eco, n) in [(Eco::Npm, "preact"), (Eco::Npm, "react"), (Eco::Npm, "colors"), (Eco::Npm, "color"), (Eco::Npm, "pg"), (Eco::Crates, "serde_json"), (Eco::Npm, "left-pad")] {
        assert_eq!(packages::near_popular(eco, n, &none), None, "{n}");
    }
    let own = BTreeSet::from(["tokii".to_string()]);
    assert_eq!(packages::near_popular(Eco::Crates, "tokii", &own), None, "the repo's own crate");
}

#[tokio::test]
async fn j_p_popular_and_own_names_are_never_looked_up() {
    let _l = port_lock().await;
    let mock = registry(vec![]).await;
    let r = regs(&mock.base, Utc::now());
    let ws = manifest("Cargo.toml", "added", "", "[package]\nname = \"kwant-core\"\n[workspace.dependencies]\nkwant-core = \"0.7\"\nserde = \"1\"\ntokio = \"1\"\n");
    let (t, f) = packages::run(&r, std::slice::from_ref(&ws.0), &BTreeMap::from([(ws.0.path.clone(), ws.1)])).await;
    assert_eq!((t.state, f.len()), (State::Pass, 0), "{t:?}");
    assert!(mock.requests().is_empty(), "{:?}", mock.requests());
}

#[tokio::test]
async fn j_p_only_url_encoded_names_are_sent_never_source() {
    let _l = port_lock().await;
    let mock = registry(vec![]).await;
    let r = regs(&mock.base, Utc::now());
    let lookalike = format!("l{}dash", '\u{043e}'); // a Cyrillic o
    let text = format!(
        r#"{{"name": "demo-web", "description": "SOURCE_MARKER", "dependencies": {{"../../admin": "1", "{lookalike}": "1", "a b?c#d": "1", "@demo/x%2F..": "1", "..": "1"}}}}"#
    );
    let (f, t) = manifest("package.json", "added", "", &text);
    let (tile, findings) = packages::run(&r, std::slice::from_ref(&f), &BTreeMap::from([(f.path.clone(), t)])).await;
    assert_eq!(tile.state, State::Hit);
    assert_eq!(tile.missing, 5, "{findings:?}");
    let paths: BTreeSet<String> = mock.requests().iter().map(|q| q.path.clone()).collect();
    let want: BTreeSet<String> = ["/npm/..%2F..%2Fadmin", "/npm/l%D0%BEdash", "/npm/a%20b%3Fc%23d", "/npm/@demo%2Fx%252F.."].iter().map(|s| s.to_string()).collect();
    assert_eq!(paths, want, "`..` alone is never sent");
    for q in mock.requests() {
        assert_eq!(q.method, "GET");
        assert!(q.body.is_empty());
        assert!(!q.head.contains("SOURCE_MARKER") && !q.head.contains("demo-web") && !q.head.contains("package.json"), "{}", q.head);
    }
    let odd = findings.iter().find(|f| f.title.contains(&lookalike)).unwrap();
    assert!(odd.detail.contains("look-alike"), "{}", odd.detail);
}

#[tokio::test]
async fn j_p_lookups_stop_at_the_cap_and_the_budget() {
    let _l = port_lock().await;
    let now = Utc::now();
    let mock = serve(MOCK_PORT, move |r| {
        if r.path.starts_with("/npm-downloads/") {
            Reply::json(200, json!({"downloads": 10_000}))
        } else {
            Reply::json(200, npm_doc(now - Days::days(500))).slow(Duration::from_millis(150))
        }
    })
    .await;
    let deps: Vec<String> = (0..30).map(|i| format!("\"demo-many-{i}\": \"1\"")).collect();
    let (f, t) = manifest("package.json", "added", "", &format!("{{\"dependencies\": {{{}}}}}", deps.join(",")));
    let contents = BTreeMap::from([(f.path.clone(), t)]);
    let r = Registries { budget: Duration::from_secs(60), ..regs(&mock.base, now) };
    let (tile, _) = packages::run(&r, std::slice::from_ref(&f), &contents).await;
    assert_eq!(tile.state, State::NotRun);
    assert!(tile.reason.as_deref().unwrap().contains("more than 25"), "{:?}", tile.reason);
    let first_run = mock.requests().iter().filter(|q| !q.path.starts_with("/npm-downloads/")).count();
    assert_eq!(first_run, packages::MAX_LOOKUPS);
    let r = Registries { budget: Duration::from_millis(500), ..regs(&mock.base, now) };
    let (tile, _) = packages::run(&r, &[f], &contents).await;
    assert_eq!(tile.state, State::NotRun);
    assert!(tile.reason.as_deref().unwrap().contains("ran past"), "{:?}", tile.reason);
}

#[test]
fn j_p_the_base_is_rebuilt_by_undoing_the_patch() {
    let head = "a\nb\nNEW\nc\nd\ne\nf\nADDED2\ng";
    let patch = "@@ -1,4 +1,4 @@\n a\n b\n-OLD\n+NEW\n c\n@@ -6,2 +6,3 @@\n e\n f\n+ADDED2";
    assert_eq!(packages::base_text(head, patch).unwrap(), "a\nb\nOLD\nc\nd\ne\nf\ng");
    assert!(packages::base_text("x\ny", "@@ -1,1 +1,1 @@\n-q\n+not-x").is_none(), "a patch that doesn't fit the head");
    // A manifest whose base can't be rebuilt reads not_run, not pass
    let f = diff("Cargo.toml", "modified", Some("@@ -1,1 +1,2 @@\n [dependencies]\n+x = \"1\""));
    let a = packages::added(&[f], &BTreeMap::from([("Cargo.toml".to_string(), "[package]\nname = \"other\"\n".to_string())]));
    assert!(a.deps.is_empty() && a.skipped[0].contains("couldn't be rebuilt"), "{a:?}");
}

#[tokio::test]
async fn j_p_an_unreadable_manifest_reads_not_run() {
    let f = diff("package.json", "modified", Some("@@ -1 +1 @@\n-{}\n+{\"dependencies\": {\"x\": \"1\"}}"));
    let (t, _) = packages::run(&Registries::all_at("http://127.0.0.1:9478"), &[f], &BTreeMap::new()).await;
    assert_eq!(t.state, State::NotRun);
    assert!(t.reason.as_deref().unwrap().contains("couldn't be read"));
}

#[test]
fn security_json_keeps_the_plan_shape_and_adds_reason_and_missing() {
    let s = Security { secrets: Tile::not_run("gitleaks didn't finish within 60 seconds"), packages: Tile { state: State::Hit, count: 2, missing: 1, reason: None }, findings: vec![] };
    assert_eq!(
        s.card_json(),
        json!({"secrets": {"state": "not_run", "count": 0, "missing": 0, "reason": "gitleaks didn't finish within 60 seconds"},
               "packages": {"state": "hit", "count": 2, "missing": 1}, "osv": {"state": "later"}, "semgrep": {"state": "later"}})
    );
    let back: Security = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    let old: Tile = serde_json::from_value(json!({"state": "pass", "count": 0})).unwrap();
    assert_eq!(old.state, State::Pass);
}

#[tokio::test]
async fn file_text_is_fetched_by_encoded_path_at_the_head_sha_and_never_outside_the_repo() {
    use crate::zombie::github::{GithubConfig, GithubSource};
    let _l = port_lock().await;
    let mock = serve(MOCK_PORT, |r| {
        use base64::Engine;
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        match r.path.as_str() {
            "/repos/demo/app/contents/web/package.json?ref=0123456789abcdef0123456789abcdef01234567" => {
                // GitHub wraps the base64 at 60 characters
                let body = b64("{\"dependencies\": {\"left-pad\": \"1\"}}");
                Reply::json(200, json!({"type": "file", "encoding": "base64", "content": format!("{}\n{}", &body[..20], &body[20..])}))
            }
            "/repos/demo/app/contents/docs/a%20b%23c.md?ref=0123456789abcdef0123456789abcdef01234567" => {
                Reply::json(200, json!({"type": "file", "encoding": "base64", "content": b64("text")}))
            }
            "/repos/demo/app/contents/src?ref=0123456789abcdef0123456789abcdef01234567" => Reply::json(200, json!([{"type": "file"}])),
            "/repos/demo/app/contents/img.bin?ref=0123456789abcdef0123456789abcdef01234567" => {
                Reply::json(200, json!({"type": "file", "encoding": "base64", "content": base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe, 0x00])}))
            }
            _ => Reply::json(404, json!({"message": "Not Found"})),
        }
    })
    .await;
    let gh = GithubSource::new("demo", GithubConfig { api_base: mock.base.clone() }, zeroize::Zeroizing::new("demo-token".into()));
    let sha = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(gh.dev_file_at("demo/app", "web/package.json", sha).await.unwrap().as_deref(), Some("{\"dependencies\": {\"left-pad\": \"1\"}}"));
    assert_eq!(gh.dev_file_at("demo/app", "docs/a b#c.md", sha).await.unwrap().as_deref(), Some("text"));
    assert_eq!(gh.dev_file_at("demo/app", "src", sha).await.unwrap(), None, "a folder");
    assert_eq!(gh.dev_file_at("demo/app", "img.bin", sha).await.unwrap(), None, "not UTF-8");
    assert_eq!(gh.dev_file_at("demo/app", "gone.rs", sha).await.unwrap(), None, "404");
    let before = mock.requests().len();
    for p in ["../../../user/repos", "a/../b", "./x", "a//b", ""] {
        assert_eq!(gh.dev_file_at("demo/app", p, sha).await.unwrap(), None, "{p}");
    }
    assert!(gh.dev_file_at("demo/app", "x", "main?evil=1").await.is_err(), "the ref must be a plain sha or name");
    assert_eq!(mock.requests().len(), before, "a path that leaves the repo's tree is never sent");
}

// ---------------------------------------------------------------------------------------------
// #316 F-A: gitleaks' built-in path allowlist can't hide an added line

#[test]
fn f_a_every_file_also_gets_a_neutral_name_and_both_copies_map_back() {
    let [r, n] = gitleaks::tree_paths_for_test(3, "node_modules/demo/index.js");
    assert_eq!(r, std::path::Path::new("r/3/node_modules/demo/index.js"));
    assert_eq!(n, std::path::Path::new("n/3.txt"));
    for p in ["package-lock.json", "assets/logo.svg", "docs/gitleaks.toml.md", "vendor/x/go.sum"] {
        let [_, n] = gitleaks::tree_paths_for_test(0, p);
        let s = n.to_string_lossy();
        assert!(!s.contains("lock") && !s.contains("svg") && !s.contains("gitleaks") && !s.contains("vendor"), "{s}");
    }
    let files = vec![target("a.rs", &["x".into()]), target("node_modules/demo/index.js", &["x".into(), "y".into()])];
    let report = json!([
        {"RuleID": "aws-access-token", "File": "/w/tree/n/1.txt", "StartLine": 2, "EndLine": 2},
        {"RuleID": "aws-access-token", "File": "/w/tree/r/1/node_modules/demo/index.js", "StartLine": 2, "EndLine": 2},
        {"RuleID": "github-pat", "File": "/w/tree/r/0/a.rs", "StartLine": 1, "EndLine": 1},
        {"RuleID": "github-pat", "File": "/w/tree/n/9.txt", "StartLine": 1, "EndLine": 1},
        {"RuleID": "github-pat", "File": "/w/tree/1/a.rs", "StartLine": 1, "EndLine": 1}
    ]);
    let hits = gitleaks::parse_report_for_test(report.to_string().as_bytes(), &files).unwrap();
    let got: Vec<(&str, u64, &str)> = hits.iter().map(|h| (h.file.as_str(), h.start, h.rule.as_str())).collect();
    // The two copies of one line are one hit; an index past the list or outside r/ and n/ names no file
    assert_eq!(got, vec![("", 1, "github-pat"), ("a.rs", 1, "github-pat"), ("node_modules/demo/index.js", 2, "aws-access-token")]);
}

#[tokio::test]
#[ignore = "needs KWANT_TEST_GITLEAKS (the real binary); CI has no network to fetch it"]
async fn f_a_real_gitleaks_finds_and_hides_keys_in_paths_its_allowlist_skips() {
    let Some(bin) = real_bin() else { return };
    let _l = port_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (archive, a, b) = fake_release(&std::fs::read(&bin).unwrap());
    let mock = serve(MOCK_PORT, move |r| if r.path == "/gl.tar.gz" { Reply::bytes(200, archive.clone()) } else { Reply::bytes(404, vec![]) }).await;
    let c = cfg(dir.path(), &mock.base, &a, &b);
    let (aws, ghp) = (fake_aws_key(), fake_github_token());
    // Only files gitleaks' default config allowlists by path: a PR like this must not read pass
    let cases = [
        ("node_modules/demo/index.js", format!("const awsKey = \"{aws}\";"), aws.clone()),
        ("assets/logo.svg", format!("<!-- aws_access_key_id = \"{aws}\" -->"), aws.clone()),
        ("docs/gitleaks.toml.md", format!("aws_access_key_id = \"{aws}\""), aws.clone()),
        ("package-lock.json", format!("      \"resolved\": \"https://demo:{ghp}@npm.pkg.github.com/demo/-/demo-1.0.0.tgz\","), ghp.clone()),
    ];
    let mut files: Vec<GhFile> = cases.iter().map(|(p, line, _)| gh(p, "added", Some(&added_patch(&["// demo", line])))).collect();
    let sec = scan_and_hide(&c, &mut files, BTreeMap::new(), None).await;
    assert_eq!(sec.secrets.state, State::Hit, "{:?}", sec.secrets);
    assert!(sec.secrets.count >= 4, "{:?}", sec.secrets);
    for ((p, _, value), f) in cases.iter().zip(&files) {
        assert!(sec.findings.iter().any(|x| x.file == *p && x.line == Some(2)), "no finding in {p}: {:?}", sec.findings);
        let patch = f.patch.as_deref().unwrap();
        assert!(patch.contains(&format!("+{HIDDEN_LINE}")) && !patch.contains(value.as_str()), "{p} not hidden: {patch}");
        assert!(patch.contains("+// demo"), "{p}: only the flagged line is hidden");
    }
}
