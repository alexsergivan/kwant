use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use reqwest::Client;
use crate::mcp::session::{Caller, McpToolSession};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn is_ollama(model: &str) -> bool {
    model == "local_ollama" || model.starts_with("ollama/")
}

/// "local_ollama" is Kwant's routing id, not something Ollama can run; resolve it to a pulled model.
async fn api_model_name(target_model: &str, url: &str) -> String {
    if let Some(m) = target_model.strip_prefix("ollama/") {
        return m.to_string();
    }
    if target_model == "local_ollama" {
        let root = url.trim_end_matches("/v1/chat/completions");
        return super::first_ollama_model(root).await.unwrap_or_else(|| "llama3.2".into());
    }
    target_model.to_string()
}

struct ChatRequest<'a> {
    url: &'a str,
    api_key: Option<&'a str>,
    model: &'a str,
    temperature: f64,
    max_tokens: i64,
    /// Model calls allowed this turn; each round of MCP tool calls costs one.
    max_calls: usize,
}

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
        if is_ollama(model) {
            let root = super::ollama_root(self.endpoint.as_deref());
            return (format!("{root}/v1/chat/completions"), None);
        }
        if let Some(ep) = &self.endpoint {
            return (ep.clone(), self.api_key.clone());
        }

        if model.contains("claude") {
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

impl ApiDriver {
    async fn post_chat(&self, request: &ChatRequest<'_>, body: &Value) -> Result<Value, String> {
        let mut req = self.client.post(request.url).json(body);
        if let Some(key) = request.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        let resp = req.send().await.map_err(|e| format!("connection failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("API returned {}", resp.status()));
        }
        resp.json::<Value>().await.map_err(|e| format!("invalid response: {e}"))
    }

    /// Calls the model, running any MCP tool calls it asks for and calling it again with the
    /// results, up to `max_calls` model calls. Returns the final text and tokens used, or
    /// `None` when the very first call fails (the caller then falls back to the offline reply).
    async fn chat_loop(
        &self,
        request: &ChatRequest<'_>,
        mut messages: Vec<Value>,
        session: Option<&McpToolSession>,
        caller: &Caller,
        event_tx: &mpsc::Sender<RuntimeEvent>,
    ) -> Option<(String, usize)> {
        let tools = session.map(|s| s.openai_tools()).unwrap_or_default();
        let max_calls = request.max_calls.max(1);
        let mut tokens = 0usize;
        for round in 0..max_calls {
            let mut body = json!({
                "model": request.model,
                "messages": messages,
                "stream": false,
                "temperature": request.temperature,
                "max_tokens": request.max_tokens,
            });
            if !tools.is_empty() {
                body["tools"] = json!(tools);
            }
            let reply = match self.post_chat(request, &body).await {
                Ok(v) => v,
                Err(e) if round == 0 => {
                    tracing::warn!(error = %e, "API call failed, using arcade emulation");
                    return None;
                }
                Err(e) => return Some((format!("⚠️ The model request failed after running tools: {e}"), tokens)),
            };
            tokens += reply["usage"]["total_tokens"].as_u64().unwrap_or(45) as usize;
            let message = &reply["choices"][0]["message"];
            let content = message["content"]
                .as_str()
                .or_else(|| reply["content"][0]["text"].as_str())
                .map(str::to_string);
            let calls = message["tool_calls"].as_array().filter(|c| !c.is_empty());

            let (Some(calls), Some(session)) = (calls, session) else {
                return Some((content.unwrap_or_else(|| "No response content".into()), tokens));
            };
            if round + 1 == max_calls {
                let note = format!("(Stopped: reached the limit of {max_calls} model calls for MCP tools this turn.)");
                let text = match content.filter(|c| !c.trim().is_empty()) {
                    Some(c) => format!("{c}\n\n{note}"),
                    None => note,
                };
                return Some((text, tokens));
            }

            if let Some(c) = content.as_deref().filter(|c| !c.trim().is_empty()) {
                let _ = event_tx.send(RuntimeEvent::ThinkingDelta(c.to_string())).await;
            }
            let mut assistant = json!({"role": "assistant", "content": content.clone().unwrap_or_default(), "tool_calls": calls});
            // Some servers return arguments as an object; the API format is a JSON string
            for call in assistant["tool_calls"].as_array_mut().into_iter().flatten() {
                if let Some(args) = call["function"].get("arguments").filter(|a| !a.is_string()).cloned() {
                    call["function"]["arguments"] = json!(args.to_string());
                }
                if call["id"].as_str().map_or(true, str::is_empty) {
                    call["id"] = json!(format!("call_{}", uuid::Uuid::new_v4().simple()));
                }
            }
            let calls = assistant["tool_calls"].as_array().cloned().unwrap_or_default();
            messages.push(assistant);

            for call in &calls {
                let id = call["id"].as_str().unwrap_or_default();
                let name = call["function"]["name"].as_str().unwrap_or_default();
                let raw_args = call["function"]["arguments"].as_str().unwrap_or("{}");
                let shown_args = serde_json::from_str::<Value>(raw_args).unwrap_or_else(|_| json!(raw_args));
                let _ = event_tx
                    .send(RuntimeEvent::ToolCallPending { id: id.to_string(), name: name.to_string(), args: shown_args })
                    .await;
                let outcome = session.call(caller, id, name, raw_args).await;
                let _ = event_tx
                    .send(RuntimeEvent::ToolCallCompleted {
                        id: id.to_string(),
                        result: json!({
                            "status": outcome.status,
                            "content": crate::mcp::truncate_text(&outcome.text, crate::mcp::RECORD_RESULT_MAX_CHARS),
                        }),
                    })
                    .await;
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": outcome.text}));
            }
        }
        None
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
        if api_key.is_some() || is_ollama(target_model) {
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

            let model = api_model_name(target_model, &url).await;
            // The Anthropic Messages API has a different tool format; it is not given MCP tools
            let mcp = input.mcp.as_ref().filter(|_| !url.ends_with("/v1/messages"));
            if input.mcp.is_some() && mcp.is_none() {
                let _ = event_tx
                    .send(RuntimeEvent::ThinkingDelta("MCP tools aren't offered on the Anthropic API endpoint; running without them.".into()))
                    .await;
            }
            let session = match mcp {
                Some(ctx) => {
                    let cwd = input.workspace_dir.as_deref().filter(|d| d.exists());
                    let (session, problems) = McpToolSession::open(ctx, cwd).await;
                    for problem in problems {
                        let _ = event_tx.send(RuntimeEvent::ThinkingDelta(format!("MCP server unavailable: {problem}"))).await;
                    }
                    Some(session)
                }
                None => None,
            };
            let request = ChatRequest {
                url: &url,
                api_key: api_key.as_deref(),
                model: &model,
                temperature: input.kwant.temperature,
                max_tokens,
                max_calls: mcp.map(|c| c.max_tool_iterations).unwrap_or(1),
            };
            let caller = Caller {
                kwant_id: input.kwant.id.clone(),
                kwant_name: input.kwant.name.clone(),
                turn_id: uuid::Uuid::new_v4().to_string(),
            };
            let outcome = self.chat_loop(&request, messages, session.as_ref(), &caller, &event_tx).await;
            if let Some(session) = session {
                session.shutdown().await;
            }
            if let Some((text, tokens)) = outcome {
                let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens,
                        duration_ms: start.elapsed().as_millis() as u64,
                    })
                    .await;
                return Ok(TurnOutput {
                    text,
                    reasoning: Some(format!("Model: {} [Effort: {}]", target_model, effort)),
                    tokens_used: tokens,
                });
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
