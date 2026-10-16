use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct ApiDriver {
    client: Client,
    api_key: Option<String>,
    endpoint: Option<String>,
}

impl ApiDriver {
    pub fn new(api_key: Option<String>, endpoint: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            endpoint,
        }
    }

    fn resolve_endpoint_and_key(&self, model: &str) -> (String, Option<String>) {
        if let Some(ep) = &self.endpoint {
            return (ep.clone(), self.api_key.clone());
        }

        if model == "local_ollama" || model.starts_with("ollama/") {
            let base = std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            (format!("{}/chat/completions", base.trim_end_matches('/')), None)
        } else if model.contains("claude") {
            let key = self
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
            ("https://api.anthropic.com/v1/messages".to_string(), key)
        } else if model.contains("grok") {
            let key = self
                .api_key
                .clone()
                .or_else(|| std::env::var("XAI_API_KEY").ok());
            ("https://api.x.ai/v1/chat/completions".to_string(), key)
        } else {
            let key = self
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok());
            ("https://api.openai.com/v1/chat/completions".to_string(), key)
        }
    }
}

#[async_trait]
impl Driver for ApiDriver {
    async fn stream_turn(
        &self,
        input: TurnInput,
        event_tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<TurnOutput, DriverError> {
        let start = Instant::now();
        let target_model = input
            .model_override
            .as_deref()
            .unwrap_or(&input.kwant.preferred_model);

        let (url, api_key) = self.resolve_endpoint_and_key(target_model);

        // If target is a real live API endpoint with API key or local Ollama, call HTTP streaming
        if api_key.is_some() || target_model == "local_ollama" {
            let system_prompt = input.format_system_prompt();

            let mut messages = vec![json!({"role": "system", "content": system_prompt})];
            for msg in &input.history {
                let (role, content) = match msg.sender_role.as_str() {
                    "kwant" if msg.sender_id == input.kwant.name => ("assistant", msg.text.clone()),
                    // Teammates' replies are context from other speakers, not this kwant's own turns
                    "kwant" => ("user", format!("[{}]: {}", msg.sender_id, msg.text)),
                    "system" => ("system", msg.text.clone()),
                    _ => ("user", msg.text.clone()),
                };
                messages.push(json!({"role": role, "content": content}));
            }
            messages.push(json!({"role": "user", "content": input.prompt}));

            let effort = input.effort_level.as_deref().unwrap_or("medium");
            let max_tokens = match effort {
                "low" => 512,
                "high" => 4096,
                _ => input.kwant.max_tokens,
            };

            let body = json!({
                "model": target_model,
                "messages": messages,
                "stream": false, // Non-stream fallback when parsing simple JSON or stream
                "temperature": input.kwant.temperature,
                "max_tokens": max_tokens,
            });

            let mut req = self.client.post(&url).json(&body);
            if let Some(key) = api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }

            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json_body) = resp.json::<serde_json::Value>().await {
                        let text = json_body["choices"][0]["message"]["content"]
                            .as_str()
                            .or_else(|| json_body["content"][0]["text"].as_str())
                            .unwrap_or("No response content")
                            .to_string();

                        let tokens = json_body["usage"]["total_tokens"].as_u64().unwrap_or(45) as usize;

                        let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
                        let duration = start.elapsed().as_millis() as u64;
                        let _ = event_tx
                            .send(RuntimeEvent::TurnComplete {
                                tokens,
                                duration_ms: duration,
                            })
                            .await;

                        return Ok(TurnOutput {
                            text,
                            reasoning: Some(format!("Model: {} [Effort: {}]", target_model, effort)),
                            tokens_used: tokens,
                        });
                    }
                }
                Ok(resp) => {
                    tracing::warn!(status = %resp.status(), "API returned non-success, using arcade emulation");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "API connection failed, using arcade emulation");
                }
            }
        }

        // Offline / Sandbox Mode: Inform user that no API key was configured
        let _ = event_tx
            .send(RuntimeEvent::ThinkingDelta(format!(
                "[{}] Searching mana link for {}...",
                input.kwant.name, target_model
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        let response_text = format!(
            "⚔️ [{}] \"No active API key or CLI connection detected for '{}'. Please enter your API key in ⚙️ Settings or authenticate your local CLI (claude / grok / codex). Received command: '{}'\"",
            input.kwant.name, target_model, input.prompt
        );

        // Stream tokens in chunks
        for word in response_text.split(' ') {
            let chunk = format!("{} ", word);
            let _ = event_tx.send(RuntimeEvent::TextDelta(chunk)).await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        let tokens = response_text.len() / 4 + 10;
        let duration = start.elapsed().as_millis() as u64;

        let _ = event_tx
            .send(RuntimeEvent::TurnComplete {
                tokens,
                duration_ms: duration,
            })
            .await;

        Ok(TurnOutput {
            text: response_text,
            reasoning: Some(format!("Model: {} (Unauthenticated)", target_model)),
            tokens_used: tokens,
        })
    }
}
