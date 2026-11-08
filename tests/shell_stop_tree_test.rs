//! Stop ends a bot command's whole process tree (Windows plan decision 3: a Job Object per turn;
//! on Mac and Linux, the process group). The command starts a child bash that starts a grandchild;
//! both keep appending to a file until they are gone. Runs on every OS; on windows-latest it is the
//! check that the Job Object holds the tree.
use kwnt::tools::shell::execute_shell;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[tokio::test]
async fn stop_ends_the_child_and_the_grandchild() {
    let tmp = tempdir().unwrap();
    // Relative names: the command runs in the workspace, and Git bash on Windows would read the
    // backslashes of an absolute path as escapes
    let args = json!({
        "command": "bash -c '(while :; do echo g >> grandchild.log; sleep 0.2; done) & while :; do echo c >> child.log; sleep 0.2; done'",
        "timeout_secs": 120
    });
    let (child, grandchild) = (tmp.path().join("child.log"), tmp.path().join("grandchild.log"));
    let dir = tmp.path().to_path_buf();
    let task = tokio::spawn(async move { execute_shell(&dir, &args).await });

    // Both are running: each file grows
    let started = std::time::Instant::now();
    while (size(&child) == 0 || size(&grandchild) == 0) && started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(size(&child) > 0, "the child never started");
    assert!(size(&grandchild) > 0, "the grandchild never started");

    // Stop: the turn is cancelled, which drops the command
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (c, g) = (size(&child), size(&grandchild));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(size(&child), c, "the child kept running after Stop");
    assert_eq!(size(&grandchild), g, "the grandchild kept running after Stop");
}
