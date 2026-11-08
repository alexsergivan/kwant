//! S-8 (Ada's review of #446, M1): the five host readers of a bot folder that used to check a
//! path and then open it again by path, raced by a "bot" that keeps swapping the file for a
//! symlink to a host file and for a FIFO, and its folder for a symlink to a host folder. Each
//! reader sees the real file or nothing: never the canary's bytes or hash, and never hangs.

use super::attachments::inside_workspace;
use super::goals_remediation::read_report;
use super::missions::checks::{self, hash_open, open_inside, Seen};
use super::missions::graph::hash_file;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CANARY: &str = "CANARY-host-secret-0f3a";

fn mkfifo(p: &Path) {
    let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: a valid C string
    unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
}

/// Keeps replacing `dir/x` (via a temp name and rename(2)) with a real file, a symlink to the
/// outside canary, then a FIFO.
fn rotate_file(dir: PathBuf, outside: PathBuf, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let (tmp, x) = (dir.join(".swap"), dir.join("x"));
        let mut i = 0u64;
        while !stop.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&tmp);
            match i % 3 {
                0 => drop(std::fs::write(&tmp, "mine")),
                1 => drop(symlink(outside.join("x"), &tmp)),
                _ => mkfifo(&tmp),
            }
            let _ = std::fs::rename(&tmp, &x);
            i += 1;
        }
    })
}

/// Keeps swapping `root/sub` between the real folder and a symlink to the outside folder.
fn swap_folder(root: PathBuf, outside: PathBuf, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut real = true;
        while !stop.load(Ordering::Relaxed) {
            if real {
                let _ = std::fs::rename(root.join("sub"), root.join("sub_parked"));
                let _ = symlink(&outside, root.join("sub"));
            } else {
                let _ = std::fs::remove_file(root.join("sub"));
                let _ = std::fs::rename(root.join("sub_parked"), root.join("sub"));
            }
            real = !real;
        }
    })
}

async fn run_readers(root: &Path, rel: &str, canary_sha: &str, rounds: usize) -> usize {
    let folder = root.to_str().unwrap();
    let mut real_reads = 0;
    for _ in 0..rounds {
        // H6 share_file
        if let Some((mut f, _)) = inside_workspace(root, rel) {
            let mut s = String::new();
            if f.read_to_string(&mut s).is_ok() {
                assert!(!s.contains(CANARY), "share_file read the host file");
                real_reads += 1;
            }
        }
        // H7 task_attach_evidence: the same open and hash the tool runs
        if let Ok(Ok(f)) = open_inside(root, rel) {
            if let Ok((sha, _)) = hash_open(f) {
                assert_ne!(sha, canary_sha, "evidence hashed the host file");
            }
        }
        // H8 the file checks
        let mut seen = Seen::default();
        if let Ok((has, out)) = checks::file("file_contains", rel, &json!({"text": "CANARY"}), root, &mut seen).await {
            assert!(!has, "file_contains saw the host file: {out}");
        }
        if let Ok((ok, out)) = checks::file("file_sha256", rel, &json!({"sha256": canary_sha}), root, &mut seen).await {
            assert!(!ok, "file_sha256 matched the host file: {out}");
        }
        let _ = checks::file("file_exists", rel, &json!({}), root, &mut seen).await;
        // The Investigate report
        if let Some(text) = read_report(folder, rel).await {
            assert!(!text.contains(CANARY), "read_report read the host file");
        }
        // The graph scan
        if let Ok(Some(h)) = hash_file(root.to_path_buf(), rel.to_string()).await {
            assert_ne!(h, canary_sha, "the graph scan hashed the host file");
        }
    }
    real_reads
}

#[test]
fn the_five_host_readers_racing_a_swapper_never_read_the_target_or_hang() {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    // On a separate thread: a reader that blocks on a FIFO fails the test instead of hanging it
    std::thread::spawn(move || {
        let t = tempfile::tempdir().unwrap();
        // Resolved: the readers compare resolved paths (macOS's /var is a link)
        std::fs::create_dir_all(t.path().join("root/sub")).unwrap();
        let (root, outside) = (t.path().join("root").canonicalize().unwrap(), t.path().join("outside"));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x"), CANARY).unwrap();
        std::fs::write(root.join("x"), "mine").unwrap();
        std::fs::write(root.join("sub/x"), "mine").unwrap();
        let canary_sha: String = Sha256::digest(CANARY.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let swappers = [
            rotate_file(root.clone(), outside.clone(), stop.clone()),
            // `sub/x` stays a real file; `sub` itself turns into a symlink to the outside folder
            swap_folder(root.clone(), outside.clone(), stop.clone()),
        ];
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let seen = rt.block_on(async {
            let a = run_readers(&root, "x", &canary_sha, 400).await;
            let b = run_readers(&root, "sub/x", &canary_sha, 400).await;
            (a, b)
        });
        stop.store(true, Ordering::Relaxed);
        for s in swappers {
            s.join().unwrap();
        }
        // Not a requirement, just that the race ran both ways
        eprintln!("share_file read the real file {} of 400 times at x, {} at sub/x", seen.0, seen.1);
        let _ = done_tx.send(());
    });
    match done_rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("a reader blocked (a FIFO?) for 120 s"),
        // The thread panicked: an assertion above failed
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("a reader read the host file (see the panic above)"),
    }
}
