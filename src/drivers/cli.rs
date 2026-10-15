use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

pub fn find_cli_binary(name: &str) -> Option<String> {
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
        let mut codex_out_file: Option<String> = None;

        let effort = input.effort_level.as_deref().unwrap_or("medium");

        let _ = event_tx
            .send(RuntimeEvent::ThinkingDelta(format!(
                "Invoking local {} CLI (effort: {}, binary: {})...",
                self.tool_kind, effort, bin_path
            )))
            .await;

        let system_prompt = input.format_system_prompt();

        match self.tool_kind.as_str() {
            "codex" => {
                let out_file = format!("/tmp/kwnt_codex_{}.txt", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                let codex_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                let full_prompt = if input.team_context.is_some() {
                    format!("[System Context: {}]\n\n{}", system_prompt, input.prompt)
                } else {
                    input.prompt.clone()
                };
                cmd.arg("exec")
                    .arg("--ephemeral")
                    .arg("--skip-git-repo-check")
                    .arg("--color")
                    .arg("never")
                    .arg("-c")
                    .arg(format!("reasoning_effort=\"{}\"", codex_effort))
                    .arg("-o")
                    .arg(&out_file)
                    .arg(&full_prompt);
                codex_out_file = Some(out_file);
            }
            "grok" => {
                if let Some(ref team) = input.team_context {
                    let desc = team.description.as_deref().unwrap_or("General team objectives");
                    cmd.arg("--rules").arg(format!("You are contributing to team @{}: {}", team.name, desc));
                }
                cmd.arg("-p").arg(&input.prompt);
            }
            _ => {
                // claude default
                if input.team_context.is_some() {
                    cmd.arg("--append-system-prompt").arg(&system_prompt);
                }
                cmd.arg("-p").arg(&input.prompt);
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
