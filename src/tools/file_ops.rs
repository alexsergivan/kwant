use super::ToolError;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_safe_path(workspace: &Path, rel_path: &str) -> Result<PathBuf, ToolError> {
    // Prevent empty paths or absolute root escapes
    let normalized = PathBuf::from(rel_path);
    if normalized.is_absolute() {
        return Err(ToolError::Security(
            "Absolute paths outside workspace are forbidden".into(),
        ));
    }

    // Resolve relative to workspace
    let target = workspace.join(normalized);

    // If target exists, canonicalize and verify workspace prefix
    if target.exists() {
        let canon_target = fs::canonicalize(&target)?;
        let canon_ws = fs::canonicalize(workspace)?;
        if !canon_target.starts_with(&canon_ws) {
            return Err(ToolError::Security(
                "Path traversal outside workspace root is blocked".into(),
            ));
        }
        Ok(canon_target)
    } else {
        // For new files, check canonical parent
        let parent = target.parent().unwrap_or(workspace);
        if parent.exists() {
            let canon_parent = fs::canonicalize(parent)?;
            let canon_ws = fs::canonicalize(workspace)?;
            if !canon_parent.starts_with(&canon_ws) {
                return Err(ToolError::Security(
                    "Path traversal outside workspace root is blocked".into(),
                ));
            }
        } else {
            // Check lexical components for ".."
            for comp in target.components() {
                if let std::path::Component::ParentDir = comp {
                    return Err(ToolError::Security(
                        "Path traversal outside workspace root is blocked".into(),
                    ));
                }
            }
        }
        Ok(target)
    }
}

pub fn write_file(workspace: &Path, args: &Value) -> Result<Value, ToolError> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'path' argument".into()))?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'content' argument".into()))?;

    let safe_path = resolve_safe_path(workspace, path_str)?;
    if let Some(parent) = safe_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&safe_path, content)?;

    Ok(json!({
        "status": "success",
        "path": path_str,
        "bytes_written": content.len()
    }))
}

pub fn read_file(workspace: &Path, args: &Value) -> Result<Value, ToolError> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("Missing 'path' argument".into()))?;

    let safe_path = resolve_safe_path(workspace, path_str)?;
    let content = fs::read_to_string(&safe_path)?;

    Ok(json!({
        "status": "success",
        "path": path_str,
        "content": content
    }))
}

pub fn list_dir(workspace: &Path, args: &Value) -> Result<Value, ToolError> {
    let path_str = args["path"].as_str().unwrap_or(".");
    let safe_path = resolve_safe_path(workspace, path_str)?;

    let mut entries = vec![];
    for entry in fs::read_dir(safe_path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        entries.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "is_dir": file_type.is_dir(),
            "bytes": entry.metadata()?.len()
        }));
    }

    Ok(json!({
        "status": "success",
        "path": path_str,
        "entries": entries
    }))
}
