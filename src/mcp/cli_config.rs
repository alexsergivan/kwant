//! Per-turn MCP config file for the claude CLI. Secrets go in the file (mode 0600, inside the
//! data dir), never in argv, and the file is removed when the guard drops.

use super::McpTurnContext;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The `--mcp-config` document: the kwant's servers plus kwnt's permission endpoint.
pub fn claude_config(ctx: &McpTurnContext) -> Value {
    let mut servers = Map::new();
    for s in &ctx.servers {
        let entry = if s.transport == "http" {
            json!({"type": "http", "url": s.url, "headers": s.headers})
        } else {
            json!({"type": "stdio", "command": s.command, "args": s.args, "env": s.env})
        };
        servers.insert(s.name.clone(), entry);
    }
    if let Some(url) = &ctx.permission_url {
        servers.insert(super::RESERVED_SERVER_NAME.to_string(), json!({"type": "http", "url": url}));
    }
    json!({ "mcpServers": servers })
}

/// Removes the config file when the turn ends, including when it is aborted.
#[derive(Debug)]
pub struct ConfigFile {
    pub path: PathBuf,
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a fresh config file for this turn in `ctx.config_dir`.
pub fn write_claude_config(ctx: &McpTurnContext) -> std::io::Result<ConfigFile> {
    create_private_dir(&ctx.config_dir)?;
    let path = ctx.config_dir.join(format!("claude-{}.json", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let guard = ConfigFile { path };
    file.write_all(serde_json::to_string_pretty(&claude_config(ctx))?.as_bytes())?;
    file.sync_all()?;
    Ok(guard)
}
