use super::ToolError;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
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

    let output = timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| ToolError::Execution("Command execution timed out".into()))?
        .map_err(|e| ToolError::Execution(format!("Failed to execute shell: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(json!({
        "command": command_str,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "success": output.status.success()
    }))
}
