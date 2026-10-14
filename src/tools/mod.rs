pub mod file_ops;
pub mod shell;
pub mod web;

use serde_json::Value;
use std::path::PathBuf;
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
}

impl ToolRunner {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&workspace_dir);
        Self { workspace_dir }
    }

    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<Value, ToolError> {
        match tool_name {
            "write_file" => file_ops::write_file(&self.workspace_dir, args),
            "read_file" => file_ops::read_file(&self.workspace_dir, args),
            "list_dir" => file_ops::list_dir(&self.workspace_dir, args),
            "execute_shell" => shell::execute_shell(&self.workspace_dir, args).await,
            "web_fetch" => web::web_fetch(args).await,
            other => Err(ToolError::UnknownTool(other.to_string())),
        }
    }
}
