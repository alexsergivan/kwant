use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

static CLI_BINARY_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>> =
    std::sync::OnceLock::new();

/// Cached lookup: resolving binaries spawns `which`, and this runs on every chat turn.
pub fn find_cli_binary(name: &str) -> Option<String> {
    let cache = CLI_BINARY_CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(name).cloned()) {
        if hit.as_deref().map_or(true, |p| Path::new(p).exists()) {
            return hit;
        }
    }
    let found = find_cli_binary_uncached(name);
    if let Ok(mut c) = cache.lock() {
        c.insert(name.to_string(), found.clone());
    }
    found
}

fn find_cli_binary_uncached(name: &str) -> Option<String> {
    // 1. Try standard which command
    if let Ok(output) = std::process::Command::new("which").arg(name).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).exists() {
                return Some(path);
            }
        }
    }

    // 2. Check ~/.local/bin and /opt/homebrew/bin
    if let Ok(home) = std::env::var("HOME") {
        let local_path = format!("{}/.local/bin/{}", home, name);
        if Path::new(&local_path).exists() {
            return Some(local_path);
        }

        // For grok / nvm node binaries
        if name == "grok" {
            let nvm_pattern = format!("{}/.nvm/versions/node", home);
            if let Ok(entries) = std::fs::read_dir(&nvm_pattern) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("bin").join("grok");
                    if candidate.exists() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    let brew_path = format!("/opt/homebrew/bin/{}", name);
    if Path::new(&brew_path).exists() {
        return Some(brew_path);
    }

    let usr_path = format!("/usr/local/bin/{}", name);
    if Path::new(&usr_path).exists() {
        return Some(usr_path);
    }

    None
}

fn toml_string(value: &str) -> String {
    serde_json::Value::String(value.to_string()).to_string()
}

/// The `mcp_servers` override for `codex exec`: only kwnt's gateway, or nothing. The gateway
/// asks the user before every call, so codex's own approval is turned off for it; its token is
/// read from an env variable so it never appears in argv.
pub fn codex_mcp_servers(ctx: Option<&crate::mcp::McpTurnContext>) -> String {
    let Some((ctx, gateway)) = ctx.and_then(|c| c.gateway.as_ref().map(|g| (c, g))) else {
        return "mcp_servers={}".into();
    };
    let slowest = ctx.servers.iter().map(|s| s.timeout_secs.max(0) as u64).max().unwrap_or(0);
    // The gateway connects to every server on codex's first tools/list
    let startup_secs = (slowest + 15).max(30);
    // A call may wait for the user's approval; codex must not give up first
    let tool_secs = (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest);
    format!(
        "mcp_servers={{{}={{url={},bearer_token_env_var={},default_tools_approval_mode=\"approve\",startup_timeout_sec={},tool_timeout_sec={}}}}}",
        crate::mcp::RESERVED_SERVER_NAME,
        toml_string(&gateway.url),
        toml_string(crate::mcp::GATEWAY_TOKEN_ENV),
        startup_secs,
        tool_secs,
    )
}

pub fn build_cli_prompt(input: &TurnInput) -> String {
    if input.history.is_empty() {
        return input.prompt.clone();
    }

    let mut parts = Vec::new();
    parts.push("### Conversation History:".to_string());
    let recent = if input.history.len() > 10 {
        &input.history[input.history.len() - 10..]
    } else {
        &input.history[..]
    };

    for msg in recent {
        let role = match msg.sender_role.as_str() {
            "user" => "User",
            // Name the speaker so a kwant can tell its own earlier replies from its teammates'
            "kwant" if !msg.sender_id.is_empty() && msg.sender_id != input.kwant.name => &msg.sender_id,
            "kwant" => "Assistant",
            "system" => "System",
            _ => &msg.sender_role,
        };
        parts.push(format!("{}: {}", role, msg.text));
    }

    parts.push(format!("\n### Current User Request:\n{}", input.prompt));
    parts.join("\n")
}

#[derive(Clone, Debug, Default)]
pub struct CliDriver {
    pub cli_binary: Option<String>,
    pub tool_kind: String, // "claude", "codex", "grok"
}

impl CliDriver {
    pub fn new(cli_binary: Option<String>, tool_kind: Option<String>) -> Self {
        let kind = tool_kind.unwrap_or_else(|| "claude".into());
        let bin = cli_binary.or_else(|| find_cli_binary(&kind));
        Self {
            cli_binary: bin,
            tool_kind: kind,
        }
    }
}

#[async_trait]
impl Driver for CliDriver {
    async fn stream_turn(
        &self,
        input: TurnInput,
        event_tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<TurnOutput, DriverError> {
        let start = Instant::now();
        let bin_path = match &self.cli_binary {
            Some(b) => b.clone(),
            None => {
                let msg = format!(
                    "⚠️ [{}] Local CLI for `{}` was not found on this system.",
                    input.kwant.name, self.tool_kind
                );
                let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens: 10,
                        duration_ms: 10,
                    })
                    .await;
                return Ok(TurnOutput {
                    text: msg,
                    reasoning: None,
                    tokens_used: 10,
                });
            }
        };

        let mut cmd = Command::new(&bin_path);
        if let Some(ref dir) = input.workspace_dir {
            if dir.exists() {
                cmd.current_dir(dir);
            }
        }
        let mut codex_out_file: Option<String> = None;
        // Lives until the CLI exits; dropping it deletes the per-turn MCP config file.
        let mut _mcp_config: Option<crate::mcp::cli_config::ConfigFile> = None;

        let effort = input.effort_level.as_deref().unwrap_or("medium");

        let _ = event_tx
            .send(RuntimeEvent::ThinkingDelta(format!(
                "Invoking local {} CLI (effort: {}, binary: {})...",
                self.tool_kind, effort, bin_path
            )))
            .await;

        let system_prompt = input.format_system_prompt();
        let effective_prompt = build_cli_prompt(&input);

        // Prevent CLI tools from spawning desktop browser windows on host
        cmd.env("BROWSER", "none");
        cmd.env("DISPLAY", "");

        if input.mcp.is_some() && self.tool_kind == "grok" {
            let _ = event_tx
                .send(RuntimeEvent::ThinkingDelta(format!(
                    "MCP servers are assigned, but kwnt can't offer MCP tools through the {} CLI yet; running without them.",
                    self.tool_kind
                )))
                .await;
        }

        match self.tool_kind.as_str() {
            "codex" => {
                let out_file = format!("/tmp/kwnt_codex_{}.txt", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                let codex_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                let full_prompt = format!("[System Context: {}]\n\n{}", system_prompt, effective_prompt);
                cmd.arg("exec")
                    .arg("--ephemeral")
                    .arg("--skip-git-repo-check")
                    .arg("--color")
                    .arg("never")
                    .arg("-c")
                    .arg(format!("reasoning_effort=\"{}\"", codex_effort))
                    // Never the user's own config.toml servers: codex exec has no hook for kwnt's
                    // approval cards. The kwant's servers come only through kwnt's gateway.
                    .arg("-c")
                    .arg(codex_mcp_servers(input.mcp.as_ref()));
                if let Some(gateway) = input.mcp.as_ref().and_then(|c| c.gateway.as_ref()) {
                    cmd.env(crate::mcp::GATEWAY_TOKEN_ENV, &gateway.token);
                }
                cmd.arg("-o")
                    .arg(&out_file)
                    .arg(&full_prompt);
                codex_out_file = Some(out_file);
            }
            "grok" => {
                let grok_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                // Without an explicit effort grok uses the user's default (often xhigh), which is far too slow for chat
                cmd.arg("--reasoning-effort").arg(grok_effort);
                cmd.arg("--no-plan").arg("--no-subagents");
                cmd.arg("--rules").arg(&system_prompt);
                cmd.arg("-p").arg(&effective_prompt);
            }
            _ => {
                // claude default
                let claude_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                cmd.arg("--effort").arg(claude_effort);
                // Skip starting the user's personal MCP servers on every chat turn
                cmd.arg("--strict-mcp-config");
                if let Some(ctx) = &input.mcp {
                    match crate::mcp::cli_config::write_claude_config(ctx) {
                        Ok(file) => {
                            cmd.arg("--mcp-config").arg(&file.path);
                            cmd.arg("--permission-prompt-tool").arg(format!(
                                "mcp__{}__{}",
                                crate::mcp::RESERVED_SERVER_NAME,
                                crate::mcp::PERMISSION_TOOL
                            ));
                            // A tool call may wait for the user's approval; claude must not give up first
                            let slowest = ctx.servers.iter().map(|s| s.timeout_secs.max(0) as u64).max().unwrap_or(0);
                            let tool_timeout_ms = (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest) * 1000;
                            cmd.env("MCP_TOOL_TIMEOUT", tool_timeout_ms.to_string());
                            cmd.env("MCP_TIMEOUT", "30000");
                            _mcp_config = Some(file);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "could not write MCP config; running without MCP");
                            let _ = event_tx
                                .send(RuntimeEvent::ThinkingDelta("MCP tools are unavailable this turn (config file error).".into()))
                                .await;
                        }
                    }
                }
                cmd.arg("--append-system-prompt").arg(&system_prompt);
                cmd.arg("-p").arg(&effective_prompt);
            }
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                let mut stdout = child.stdout.take().unwrap();
                let mut output_str = String::new();
                let mut buffer = [0u8; 1024];

                if codex_out_file.is_none() {
                    while let Ok(n) = stdout.read(&mut buffer).await {
                        if n == 0 {
                            break;
                        }
                        let chunk = String::from_utf8_lossy(&buffer[..n]).to_string();
                        let _ = event_tx.send(RuntimeEvent::TextDelta(chunk.clone())).await;
                        output_str.push_str(&chunk);
                    }
                }

                let _ = child.wait().await;

                if let Some(ref out_file) = codex_out_file {
                    if let Ok(content) = tokio::fs::read_to_string(out_file).await {
                        output_str = content;
                    }
                    let _ = tokio::fs::remove_file(out_file).await;
                    let _ = event_tx.send(RuntimeEvent::TextDelta(output_str.clone())).await;
                }

                let duration = start.elapsed().as_millis() as u64;
                let tokens = output_str.len() / 4;

                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens,
                        duration_ms: duration,
                    })
                    .await;

                Ok(TurnOutput {
                    text: output_str,
                    reasoning: Some(format!(
                        "Executed via {} CLI ({}) [Effort: {}]",
                        self.tool_kind, bin_path, effort
                    )),
                    tokens_used: tokens,
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, binary = %bin_path, "CLI binary execution failed");
                let msg = format!(
                    "⚠️ [{}] Execution error for CLI `{}`: {}",
                    input.kwant.name, bin_path, e
                );
                let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens: 15,
                        duration_ms: start.elapsed().as_millis() as u64,
                    })
                    .await;

                Ok(TurnOutput {
                    text: msg,
                    reasoning: None,
                    tokens_used: 15,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{Kwant, Message};

    #[test]
    fn test_build_cli_prompt_with_history() {
        let kwant = Kwant {
            id: "k1".into(),
            name: "Architect".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "grok".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        };
        let input = TurnInput {
            browser_api_url: None,
            thread_id: "t1".into(),
            kwant,
            prompt: "open some playlist page".into(),
            history: vec![
                Message {
                    id: "m1".into(),
                    channel_id: Some("c1".into()),
                    direct_kwant_id: None,
                    sender_role: "user".into(),
                    sender_id: "Player".into(),
                    text: "open booktun.ing".into(),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 10,
                    created_at: None,
                },
                Message {
                    id: "m2".into(),
                    channel_id: Some("c1".into()),
                    direct_kwant_id: None,
                    sender_role: "kwant".into(),
                    sender_id: "Architect".into(),
                    text: "I opened booktun.ing for you.".into(),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 20,
                    created_at: None,
                },
            ],
            model_override: None,
            effort_level: None,
            team_context: None,
            workspace_dir: None,
            sandbox_id: None,
            teammates: vec![],
            mcp: None,
        };

        let result = build_cli_prompt(&input);
        assert!(result.contains("### Conversation History:"));
        assert!(result.contains("User: open booktun.ing"));
        assert!(result.contains("Assistant: I opened booktun.ing for you."));
        assert!(result.contains("### Current User Request:\nopen some playlist page"));
    }

    #[test]
    fn test_build_cli_prompt_without_history() {
        let kwant = Kwant {
            id: "k1".into(),
            name: "Architect".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "grok".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        };
        let input = TurnInput {
            browser_api_url: None,
            thread_id: "t1".into(),
            kwant,
            prompt: "hello world".into(),
            history: vec![],
            model_override: None,
            effort_level: None,
            team_context: None,
            workspace_dir: None,
            sandbox_id: None,
            teammates: vec![],
            mcp: None,
        };

        let result = build_cli_prompt(&input);
        assert_eq!(result, "hello world");
    }
}
