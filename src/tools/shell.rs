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
    execute_shell_env(workspace, args, &[]).await
}

/// [`execute_shell`] with extra environment variables, for kwant's own callers (outcome checks
/// pass KWANT_TASK_ID and friends). Bots reach only [`execute_shell`], which adds none.
pub async fn execute_shell_env(workspace: &Path, args: &Value, env: &[(String, String)]) -> Result<Value, ToolError> {
    let command_str = args["command"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'command' argument".into()))?;

    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

    let mut cmd = Command::new(shell_program()?);
    cmd.arg("-c").arg(command_str);
    cmd.current_dir(workspace);
    for (k, v) in env {
        cmd.env(k, v);
    }
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

/// The bash that runs bot commands. On Windows this is Git for Windows' `bash.exe`, never
/// WSL's `System32\bash.exe` (plan decision 2); without Git, the command is refused with
/// [`GIT_MISSING`].
fn shell_program() -> Result<std::path::PathBuf, ToolError> {
    #[cfg(windows)]
    return git_bash().ok_or_else(|| ToolError::Execution(GIT_MISSING.into()));
    #[cfg(not(windows))]
    Ok("bash".into())
}

/// Shown when Git for Windows (and so its bash) is missing.
pub const GIT_MISSING: &str = "Bot commands need Git for Windows. Install it with: winget install --id Git.Git -e";

/// Git for Windows' `bash.exe`, looked up once: from `git --exec-path`, else the usual install
/// folders. Never WSL's bash in `C:\Windows\System32`.
#[cfg(windows)]
pub fn git_bash() -> Option<std::path::PathBuf> {
    // Only a hit is kept, so Git installed while kwnt runs is found on the next command
    static FOUND: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    if let Some(hit) = FOUND.get() {
        return Some(hit.clone());
    }
    let found = {
            // Through the one git helper (D4c): no system config, hooks or stray GIT_* here either
            let exec_path = crate::server::dev::git::host_git_command_blocking("git")
                .arg("--exec-path")
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            let env = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
            git_bash_candidates(exec_path.as_deref(), env("ProgramFiles"), env("LOCALAPPDATA"))
                .into_iter()
                .find(|p| p.is_file())
    }?;
    Some(FOUND.get_or_init(|| found).clone())
}

/// Where Git for Windows' bash may be, best first: `<git --exec-path>\..\..\..\bin\bash.exe`
/// (exec-path is `<Git>\mingw64\libexec\git-core`), then the machine-wide and per-user
/// install folders. WSL's bash is never a candidate.
#[cfg(any(windows, test))]
fn git_bash_candidates(
    exec_path: Option<&str>,
    program_files: Option<std::path::PathBuf>,
    local_app_data: Option<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Some(root) = exec_path.filter(|p| !p.is_empty()).and_then(|p| Path::new(p).ancestors().nth(3)) {
        out.push(root.join("bin").join("bash.exe"));
    }
    if let Some(pf) = program_files {
        out.push(pf.join("Git").join("bin").join("bash.exe"));
    }
    if let Some(local) = local_app_data {
        out.push(local.join("Programs").join("Git").join("bin").join("bash.exe"));
    }
    out.retain(|p| !is_wsl_bash(p));
    out
}

/// `C:\Windows\System32\bash.exe` (or SysWOW64/Sysnative) is WSL's launcher, not a bash.
#[cfg(any(windows, test))]
fn is_wsl_bash(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    ["\\windows\\system32\\", "\\windows\\syswow64\\", "\\windows\\sysnative\\"].iter().any(|d| p.contains(d))
}

#[cfg(test)]
mod windows_path_tests {
    use super::*;
    use std::path::PathBuf;

    // Forward slashes so the same paths split the same way on Linux, Mac and Windows.
    #[test]
    fn git_bash_comes_from_git_s_own_folder_first_and_never_from_wsl() {
        let got = git_bash_candidates(
            Some("C:/Program Files/Git/mingw64/libexec/git-core"),
            Some(PathBuf::from("C:/Program Files")),
            Some(PathBuf::from("C:/Users/someone/AppData/Local")),
        );
        let git = |root: &str| Path::new(root).join("bin").join("bash.exe");
        assert_eq!(
            got,
            vec![
                git("C:/Program Files/Git"),
                Path::new("C:/Program Files").join("Git").join("bin").join("bash.exe"),
                Path::new("C:/Users/someone/AppData/Local").join("Programs").join("Git").join("bin").join("bash.exe"),
            ]
        );
        assert!(git_bash_candidates(None, None, None).is_empty());
        assert!(git_bash_candidates(Some(""), None, None).is_empty());
        // A git whose folder is in System32 would point at WSL's bash: dropped
        assert!(git_bash_candidates(Some("C:/Windows/System32/a/b/c"), None, None).is_empty());
    }

    #[test]
    fn wsl_s_launcher_is_not_git_bash() {
        for wsl in ["C:\\Windows\\System32\\bash.exe", "c:/windows/sysnative/bash.exe", "C:\\WINDOWS\\SysWOW64\\bash.exe"] {
            assert!(is_wsl_bash(Path::new(wsl)), "{wsl}");
        }
        for git in ["C:\\Program Files\\Git\\bin\\bash.exe", "D:/tools/Git/bin/bash.exe"] {
            assert!(!is_wsl_bash(Path::new(git)), "{git}");
        }
    }

    #[test]
    fn missing_git_says_how_to_install_it() {
        assert!(GIT_MISSING.contains("winget install --id Git.Git -e"));
    }
}
