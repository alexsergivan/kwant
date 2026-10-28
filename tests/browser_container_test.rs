//! The VM browser container's lifecycle against real Docker: run with
//! `cargo test --test browser_container_test -- --ignored` on a host with Docker and the
//! `kwnt-browser:v2` image. Ignored by default so CI and ordinary test runs never start a container.
use kwnt::sandbox::service::{managed_container_name, BrowserService};
use std::process::Command;
use std::time::Duration;

fn docker(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Removes the container whatever the test's outcome, so a failure leaves nothing running.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) { let _ = docker(&["rm", "-f", &self.0]); }
}

#[tokio::test]
#[ignore = "needs Docker and the kwnt-browser:v2 image; starts a real container"]
async fn a_fresh_kwants_container_has_no_restart_and_labels_stops_at_shutdown_and_is_reaped_once_its_folder_is_gone() {
    std::env::set_var(kwnt::sandbox::computer::MODE_ENV, "vm");
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let name = managed_container_name(&root);
    let _cleanup = Cleanup(name.clone());

    let service = BrowserService::new(root.clone(), None);
    service.session("bot", &root).await.expect("a browser session in a new container");

    let inspect = docker(&["inspect", "--format",
        "{{.HostConfig.RestartPolicy.Name}}|{{index .Config.Labels \"kwnt.browser\"}}|{{index .Config.Labels \"kwnt.root\"}}|{{.State.Running}}",
        &name]).expect("the container exists");
    assert_eq!(inspect, format!("no|1|{}|true", root.display()));

    service.shutdown().await;
    assert_eq!(docker(&["inspect", "--format", "{{.State.Running}}", &name]).as_deref(), Some("false"));

    // its folder is still there: a stopped container is kept for kwant to start again
    let limit = Duration::from_secs(30 * 60);
    service.reap_containers(limit).await;
    assert!(docker(&["inspect", &name]).is_some(), "kept while its data folder exists");

    // the next request starts it again, with the same labels
    service.session("bot", &root).await.expect("restarted on demand");
    assert_eq!(docker(&["inspect", "--format", "{{.State.Running}}", &name]).as_deref(), Some("true"));

    drop(tmp);
    assert!(!root.exists());
    let (removed, _) = service.reap_containers(limit).await;
    assert!(removed >= 1);
    assert!(docker(&["inspect", &name]).is_none(), "removed once its data folder is gone");
}
