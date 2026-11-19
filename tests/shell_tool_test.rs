//! The shell tool stops what it started: on timeout and when its turn is cancelled.
use kwnt::tools::shell::execute_shell;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn a_timed_out_command_does_not_finish_later() {
    let tmp = tempdir().unwrap();
    let marker = tmp.path().join("marker");
    // Relative names (the command runs in the workspace): Git bash on Windows reads backslashes as escapes
    let args = json!({ "command": "sleep 3; touch marker", "timeout_secs": 1 });
    let err = execute_shell(tmp.path(), &args).await.unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!marker.exists(), "the command kept running after its timeout");
}

#[tokio::test]
async fn a_timeout_also_stops_background_jobs() {
    let tmp = tempdir().unwrap();
    let marker = tmp.path().join("grandchild");
    // A backgrounded subshell is a grandchild of the tool; it must die with bash
    let args = json!({
        "command": "(sleep 3; touch grandchild) & sleep 10",
        "timeout_secs": 1
    });
    assert!(execute_shell(tmp.path(), &args).await.is_err());
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!marker.exists(), "a background job outlived the timed-out command");
}

#[tokio::test]
async fn a_cancelled_turn_stops_the_command() {
    let tmp = tempdir().unwrap();
    let marker = tmp.path().join("cancelled");
    let args = json!({
        "command": "(sleep 3; touch cancelled) & sleep 3; touch cancelled",
        "timeout_secs": 30
    });
    let dir = tmp.path().to_path_buf();
    let task = tokio::spawn(async move { execute_shell(&dir, &args).await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!marker.exists(), "the command kept running after its turn was cancelled");
}

#[tokio::test]
async fn a_finished_command_still_returns_its_output() {
    let tmp = tempdir().unwrap();
    let args = json!({ "command": "echo out; echo err >&2; exit 3" });
    let v = execute_shell(tmp.path(), &args).await.unwrap();
    assert_eq!(v["stdout"], "out\n");
    assert_eq!(v["stderr"], "err\n");
    assert_eq!(v["exit_code"], 3);
    assert_eq!(v["success"], false);
}

// Everything a bot runs on the host is first in line when memory runs out, so a runaway build
// is stopped before kwnt is; whatever the command starts inherits it.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_bot_command_and_what_it_starts_go_first_when_memory_runs_out() {
    let tmp = tempdir().unwrap();
    let args = json!({ "command": "cat /proc/self/oom_score_adj; bash -c 'cat /proc/self/oom_score_adj'" });
    let v = execute_shell(tmp.path(), &args).await.unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    assert_eq!(v["stdout"], "1000\n1000\n", "{v}");
}

// A command killed from outside says so, and keeps exit_code -1 for existing readers. kwnt's
// own timeout is not a kill seen here (it is an error, tested above).
#[cfg(unix)]
#[tokio::test]
async fn a_command_killed_from_outside_is_told_why() {
    let tmp = tempdir().unwrap();
    let args = json!({ "command": "echo before >&2; kill -9 $$" });
    let v = execute_shell(tmp.path(), &args).await.unwrap();
    assert_eq!(v["signal"], 9, "{v}");
    assert_eq!(v["exit_code"], -1, "{v}");
    assert_eq!(v["success"], false, "{v}");
    let stderr = v["stderr"].as_str().unwrap();
    assert!(stderr.starts_with("before\n"), "{stderr}");
    assert!(stderr.contains(kwnt::tools::shell::KILLED_FROM_OUTSIDE), "{stderr}");
    assert!(!stderr.contains("ran out of memory"), "{stderr}");
}

#[tokio::test]
async fn a_normal_exit_has_no_signal() {
    let tmp = tempdir().unwrap();
    let v = execute_shell(tmp.path(), &json!({ "command": "exit 0" })).await.unwrap();
    assert!(v.get("signal").is_none(), "{v}");
    assert_eq!(v["stderr"], "", "{v}");
}
