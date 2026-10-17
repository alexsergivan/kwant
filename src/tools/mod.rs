pub mod file_ops;
pub mod sandbox;
pub mod shell;
pub mod web;

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("I/O error during tool execution: {0}")]
    Io(#[from] std::io::Error),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Invalid tool arguments: {0}")]
    InvalidArgs(String),

    #[error("Tool execution failed: {0}")]
    Execution(String),

    #[error("Unknown tool '{0}'")]
    UnknownTool(String),
}

#[derive(Clone, Debug)]
pub struct ToolRunner {
    pub workspace_dir: PathBuf,
    pub sandbox_manager: Option<Arc<crate::sandbox::SandboxManager>>,
    pub browser_controller: Option<crate::sandbox::BrowserController>,
}

impl ToolRunner {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&workspace_dir);
        Self {
            workspace_dir,
            sandbox_manager: None,
            browser_controller: None,
        }
    }

    pub fn with_sandbox(mut self, manager: Arc<crate::sandbox::SandboxManager>) -> Self {
        self.sandbox_manager = Some(manager);
        self
    }

    pub fn with_browser(mut self, controller: crate::sandbox::BrowserController) -> Self {
        self.browser_controller = Some(controller);
        self
    }

    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<Value, ToolError> {
        self.execute_in_workspace(&self.workspace_dir, tool_name, args).await
    }

    pub async fn execute_in_workspace(&self, target_dir: &std::path::Path, tool_name: &str, args: &Value) -> Result<Value, ToolError> {
        match tool_name {
            "write_file" => file_ops::write_file(target_dir, args),
            "read_file" => file_ops::read_file(target_dir, args),
            "list_dir" => file_ops::list_dir(target_dir, args),
            "execute_shell" => shell::execute_shell(target_dir, args).await,
            "web_fetch" => web::web_fetch(args).await,
            "browser_navigate" => sandbox::browser_navigate_with_controller(self.browser_controller.as_ref(), args).await,
            "browser_screenshot" => sandbox::browser_screenshot_with_controller(self.browser_controller.as_ref(), args).await,
            "browser_extract" => sandbox::browser_extract_with_controller(self.browser_controller.as_ref(), args).await,
            "browser_click" => sandbox::browser_click_with_controller(self.browser_controller.as_ref(), args).await,
            "browser_type" => sandbox::browser_type_with_controller(self.browser_controller.as_ref(), args).await,
            other => Err(ToolError::UnknownTool(other.to_string())),
        }
    }
}

pub async fn resolve_or_prepare_workspace(
    base_data_dir: &std::path::Path,
    team_name: &str,
    workspace_path: Option<&str>,
    repo_url: Option<&str>,
) -> PathBuf {
    let target_dir = if let Some(ws) = workspace_path.filter(|s| !s.trim().is_empty()) {
        let trimmed = ws.trim();
        if let Some(rest) = trimmed.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                PathBuf::from(home).join(rest)
            } else {
                PathBuf::from(trimmed)
            }
        } else {
            PathBuf::from(trimmed)
        }
    } else if repo_url.filter(|s| !s.trim().is_empty()).is_some() {
        base_data_dir.join("workspaces").join(team_name)
    } else {
        base_data_dir.join("workspace")
    };

    if let Some(repo) = repo_url.filter(|s| !s.trim().is_empty()) {
        let git_dir = target_dir.join(".git");
        if !git_dir.exists() {
            let _ = std::fs::create_dir_all(&target_dir);
            let clone_res = tokio::process::Command::new("git")
                .arg("clone")
                .arg(repo.trim())
                .arg(&target_dir)
                .output()
                .await;
            if let Err(e) = clone_res {
                tracing::warn!("Failed to run git clone for team {}: {}", team_name, e);
            }
        }
    } else {
        let _ = std::fs::create_dir_all(&target_dir);
    }

    target_dir
}
