use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

#[derive(Error, Debug)]
pub enum SandboxError {
    #[error("Docker is not available on this host: {0}")]
    DockerUnavailable(String),
    #[error("Failed to start sandbox container: {0}")]
    StartFailed(String),
    #[error("Execution inside sandbox failed: {0}")]
    ExecFailed(String),
    #[error("Execution timed out")]
    Timeout,
    #[error("Sandbox not found for team: {0}")]
    NotFound(String),
}

pub const EXECUTOR_IMAGE: &str = "kwnt-executor:v1";

/// True when the image `tag` is already built.
pub async fn image_exists(tag: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", tag])
        .kill_on_drop(true)
        .output()
        .await
        .is_ok_and(|out| out.status.success())
}

/// Builds `tag` from `files` (a `Dockerfile` and what it copies) unless it exists. The build
/// folder is private to this build, so two images never share one.
pub async fn ensure_image(tag: &str, files: &[(&str, &str)], build_args: &[(&str, &str)]) -> Result<(), SandboxError> {
    if image_exists(tag).await {
        return Ok(());
    }
    let build = std::env::temp_dir().join(format!("kwnt-image-build-{}", uuid::Uuid::new_v4().simple()));
    let result = async {
        tokio::fs::create_dir_all(&build).await?;
        for (name, content) in files {
            tokio::fs::write(build.join(name), content).await?;
        }
        let mut cmd = Command::new("docker");
        cmd.args(["build", "-t", tag]);
        for (k, v) in build_args {
            cmd.arg("--build-arg").arg(format!("{k}={v}"));
        }
        cmd.arg(&build).kill_on_drop(true).output().await
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&build).await;
    let out = result.map_err(|e| SandboxError::StartFailed(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(SandboxError::StartFailed(tail));
    }
    Ok(())
}

/// kwnt's base image for commands run in the computer (and the agent image's base).
pub async fn ensure_executor_image() -> Result<(), SandboxError> {
    ensure_image(EXECUTOR_IMAGE, &[("Dockerfile", include_str!("../../docker/executor.Dockerfile"))], &[]).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveSandbox {
    pub team_id: String,
    pub container_name: String,
    pub cdp_port: u16,
    pub workspace_path: PathBuf,
    pub is_running: bool,
}

#[derive(Clone, Debug)]
pub struct SandboxManager {
    sandboxes: Arc<Mutex<HashMap<String, ActiveSandbox>>>,
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxManager {
    pub fn new() -> Self {
        Self {
            sandboxes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn is_docker_available(&self) -> bool {
        // A stalled daemon must not leave browser controls waiting indefinitely.
        matches!(
            timeout(
                Duration::from_secs(3),
                Command::new("docker").arg("version").kill_on_drop(true).output(),
            ).await,
            Ok(Ok(output)) if output.status.success()
        )
    }

    pub async fn get_sandbox(&self, team_id: &str) -> Option<ActiveSandbox> {
        let lock = self.sandboxes.lock().await;
        lock.get(team_id).cloned()
    }

    pub async fn ensure_sandbox_running(
        &self,
        team_id: &str,
        team_name: &str,
        workspace_dir: &Path,
    ) -> Result<ActiveSandbox, SandboxError> {
        if !self.is_docker_available().await {
            return Err(SandboxError::DockerUnavailable("docker command failed".into()));
        }

        let mut lock = self.sandboxes.lock().await;
        if let Some(existing) = lock.get(team_id) {
            // Check if still running
            let inspect = Command::new("docker")
                .args(["inspect", "-f", "{{.State.Running}}", &existing.container_name])
                .output()
                .await;
            if let Ok(out) = inspect {
                if String::from_utf8_lossy(&out.stdout).trim() == "true" {
                    return Ok(existing.clone());
                }
            }
        }

        let safe_name = team_name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
        let id_suffix = if team_id.len() >= 8 { &team_id[..8] } else { team_id };
        let container_name = format!("kwnt-executor-{}-{}", safe_name, id_suffix);

        // Remove old container if exists
        let _ = Command::new("docker").args(["rm", "-f", &container_name]).output().await;

        let ws_str = workspace_dir.to_string_lossy().to_string();
        ensure_executor_image().await?;
        let run_res = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                &container_name,
                "--cpus=2.0",
                "--memory=1536m",
                "-v",
                &format!("{}:/workspace:rw", ws_str),
                "-w",
                "/workspace",
                EXECUTOR_IMAGE,
            ])
            .output()
            .await
            .map_err(|e| SandboxError::StartFailed(e.to_string()))?;

        if !run_res.status.success() {
            let err = String::from_utf8_lossy(&run_res.stderr);
            return Err(SandboxError::StartFailed(err.to_string()));
        }

        let active = ActiveSandbox {
            team_id: team_id.to_string(),
            container_name,
            cdp_port: 0,
            workspace_path: workspace_dir.to_path_buf(),
            is_running: true,
        };

        lock.insert(team_id.to_string(), active.clone());
        Ok(active)
    }

    pub async fn exec_in_sandbox(
        &self,
        container_name: &str,
        command_str: &str,
        timeout_secs: u64,
    ) -> Result<ShellOutput, SandboxError> {
        let mut cmd = Command::new("docker");
        cmd.args(["exec", "-w", "/workspace", container_name, "bash", "-c", command_str]);

        let output = timeout(Duration::from_secs(timeout_secs), cmd.output())
            .await
            .map_err(|_| SandboxError::Timeout)?
            .map_err(|e| SandboxError::ExecFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success()
            && (stderr.contains("No such container")
                || stderr.contains("is not running")
                || stderr.contains("Error response from daemon")
                || stderr.contains("Cannot connect to the Docker daemon"))
        {
            return Err(SandboxError::ExecFailed(stderr.trim().to_string()));
        }

        Ok(ShellOutput {
            command: command_str.to_string(),
            stdout,
            stderr,
            exit_code,
            success: output.status.success(),
        })
    }

    pub async fn stop_sandbox(&self, team_id: &str) -> Result<(), SandboxError> {
        let mut lock = self.sandboxes.lock().await;
        if let Some(active) = lock.remove(team_id) {
            let _ = Command::new("docker")
                .args(["rm", "-f", &active.container_name])
                .output()
                .await;
        }
        Ok(())
    }
}
