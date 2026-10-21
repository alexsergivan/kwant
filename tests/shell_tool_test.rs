//! The shell tool stops what it started: on timeout and when its turn is cancelled.
use kwnt::tools::shell::execute_shell;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn a_timed_out_command_does_not_finish_later() {
    let tmp = tempdir().unwrap();
    let marker = tmp.path().join("marker");
    let args = json!({ "command": format!("sleep 3; touch {}", marker.display()), "timeout_secs": 1 });
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
        "command": format!("(sleep 3; touch {}) & sleep 10", marker.display()),
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
        "command": format!("(sleep 3; touch {m}) & sleep 3; touch {m}", m = marker.display()),
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
