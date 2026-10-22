use super::ToolError;
use crate::drivers::cli::{own_process_group, ProcessGroup};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

pub async fn execute_shell(workspace: &Path, args: &Value) -> Result<Value, ToolError> {
    let command_str = args["command"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'command' argument".into()))?;

    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command_str);
    cmd.current_dir(workspace);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // A cancelled turn drops this future; neither bash nor anything it started may outlive it
    cmd.kill_on_drop(true);
    own_process_group(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::Execution(format!("Failed to execute shell: {}", e)))?;
    // Declared after the child, so on cancel it drops first and kills the group while bash is unreaped
    let mut group = ProcessGroup::of(&child);
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let run = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let read_out = async {
            if let Some(p) = stdout_pipe.as_mut() {
                let _ = p.read_to_end(&mut stdout).await;
            }
        };
        let read_err = async {
            if let Some(p) = stderr_pipe.as_mut() {
                let _ = p.read_to_end(&mut stderr).await;
            }
        };
        let (status, _, _) = tokio::join!(child.wait(), read_out, read_err);
        (status, stdout, stderr)
    };

    let (status, stdout, stderr) = match timeout(Duration::from_secs(timeout_secs), run).await {
        Ok(done) => done,
        Err(_) => {
            // Kill the whole group (background jobs too) before bash can be reaped and its id reused
            drop(group);
            let _ = child.kill().await;
            return Err(ToolError::Execution(format!(
                "Command timed out after {}s and was stopped, with everything it started",
                timeout_secs
            )));
        }
    };
    // Finished normally: anything it deliberately left running in the background is kept
    group.disarm();
    let status = status.map_err(|e| ToolError::Execution(format!("Failed to execute shell: {}", e)))?;

    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();
    let exit_code = status.code().unwrap_or(-1);

    Ok(json!({
        "command": command_str,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "success": status.success()
    }))
}
