use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Clone, Debug, Default)]
pub struct CliDriver {
    pub cli_binary: Option<String>,
}

impl CliDriver {
    pub fn new(cli_binary: Option<String>) -> Self {
        Self { cli_binary }
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
        let bin = self.cli_binary.as_deref().unwrap_or("claude");

        let mut cmd = Command::new(bin);
        cmd.arg("--print").arg(&input.prompt);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                let mut stdout = child.stdout.take().unwrap();
                let mut output_str = String::new();
                let mut buffer = [0u8; 1024];

                while let Ok(n) = stdout.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    let chunk = String::from_utf8_lossy(&buffer[..n]).to_string();
                    let _ = event_tx.send(RuntimeEvent::TextDelta(chunk.clone())).await;
                    output_str.push_str(&chunk);
                }

                let _ = child.wait().await;
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
                    reasoning: Some(format!("Executed via {} CLI", bin)),
                    tokens_used: tokens,
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, binary = %bin, "CLI binary execution failed");
                let msg = format!(
                    "⚠️ [{}] CLI `{}` is not installed or available on this host. ({})",
                    input.kwant.name, bin, e
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
