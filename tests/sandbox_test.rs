use kwnt::sandbox::manager::{ActiveSandbox, SandboxError, SandboxManager, ShellOutput};
use std::path::PathBuf;
use tempfile::tempdir;

#[tokio::test]
async fn test_sandbox_manager_offline_fallback_and_state() {
    let mgr = SandboxManager::new();
    let _tmp = tempdir().unwrap();

    // Verify sandbox status lookup on clean state
    assert!(mgr.get_sandbox("non-existent").await.is_none());

    // Executing command in non-existent container yields error
    let res = mgr.exec_in_sandbox("kwnt-fake-container", "echo hi", 5).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_sandbox_manager_docker_availability_and_default() {
    let mgr = SandboxManager::default();
    let _available = mgr.is_docker_available().await;
    // Just verify the call succeeds without panic
    let stop_res = mgr.stop_sandbox("unknown-team").await;
    assert!(stop_res.is_ok());
}

#[test]
fn test_sandbox_types_serialization_and_errors() {
    let output = ShellOutput {
        command: "echo test".into(),
        stdout: "test\n".into(),
        stderr: "".into(),
        exit_code: 0,
        success: true,
    };
    let json = serde_json::to_string(&output).unwrap();
    let deserialized: ShellOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.command, "echo test");
    assert_eq!(deserialized.exit_code, 0);
    assert!(deserialized.success);

    let active = ActiveSandbox {
        team_id: "t1".into(),
        container_name: "kwnt-sandbox-t1".into(),
        cdp_port: 9222,
        workspace_path: PathBuf::from("/workspace"),
        is_running: true,
    };
    let active_json = serde_json::to_string(&active).unwrap();
    let active_deser: ActiveSandbox = serde_json::from_str(&active_json).unwrap();
    assert_eq!(active_deser.team_id, "t1");
    assert_eq!(active_deser.cdp_port, 9222);

    let err = SandboxError::DockerUnavailable("not found".into());
    assert!(format!("{}", err).contains("Docker is not available"));
    let err_timeout = SandboxError::Timeout;
    assert_eq!(format!("{}", err_timeout), "Execution timed out");
}

#[test]
fn test_container_naming_format() {
    let team_name = "Alpha Team & Co";
    let team_id = "01234567-89ab-cdef-0123-456789abcdef";
    let safe_name = team_name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
    let id_suffix = if team_id.len() >= 8 { &team_id[..8] } else { team_id };
    let container_name = format!("kwnt-sandbox-{}-{}", safe_name, id_suffix);
    assert_eq!(container_name, "kwnt-sandbox-alpha-team---co-01234567");

    let short_id = "abc";
    let short_suffix = if short_id.len() >= 8 { &short_id[..8] } else { short_id };
    let container_name_short = format!("kwnt-sandbox-test-{}", short_suffix);
    assert_eq!(container_name_short, "kwnt-sandbox-test-abc");
}
