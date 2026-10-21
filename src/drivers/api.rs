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

/// The model id to send. A model picked for the turn wins. Otherwise routing ids are resolved:
/// "local_ollama" to a pulled model, and a bare family ("claude", "codex", "grok") to the
/// provider's model of the effort's tier from its own model list, since none of those names is a
/// model the provider knows. `Err` when no model can be found.
async fn api_model_name(
    target_model: &str,
    picked: Option<&str>,
    url: &str,
    api_key: Option<&str>,
    effort: &str,
) -> Result<String, String> {
    if let Some(m) = picked {
        return Ok(m.strip_prefix("ollama/").unwrap_or(m).to_string());
    }
    if let Some(m) = target_model.strip_prefix("ollama/") {
        return Ok(m.to_string());
    }
    if target_model == "local_ollama" {
        let root = url.trim_end_matches("/v1/chat/completions");
        return Ok(super::first_ollama_model(root).await.unwrap_or_else(|| "llama3.2".into()));
    }
    let engine = match target_model {
        "claude" => super::engines::EngineId::AnthropicApi,
        "codex" | "openai" => super::engines::EngineId::OpenaiApi,
        "grok" => super::engines::EngineId::XaiApi,
        model => return Ok(model.to_string()),
    };
    let listed = match api_key {
        Some(key) => list_api_models(engine, url, key).await,
        None => Vec::new(),
    };
    let tier = super::engines::Tier::from_effort(effort);
    match super::engines::choose_model(&listed, None, tier) {
        Some(m) => Ok(m.id),
        None if engine == super::engines::EngineId::AnthropicApi => Ok(DEFAULT_CLAUDE_MODEL.into()),
        None => Err(format!(
            "Couldn't find a model to run on the {}: its model list didn't load. Check the key in Space settings → Accounts.",
            engine.name()
        )),
    }
}

/// The provider's chat models, from `GET /v1/models` beside the chat endpoint `url`.
async fn list_api_models(engine: super::engines::EngineId, url: &str, key: &str) -> Vec<super::engines::EngineModel> {
    let base = url.split("/v1/").next().unwrap_or(url);
    let Ok(client) = Client::builder().timeout(Duration::from_secs(10)).build() else {
        return Vec::new();
    };
    let req = client.get(format!("{base}/v1/models"));
    let req = if is_anthropic(url) {
        req.header("x-api-key", key).header("anthropic-version", ANTHROPIC_VERSION)
    } else {
        req.bearer_auth(key)
    };
    match req.send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(body) => super::engines::parse_api_models(engine, &body),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Anthropic Messages API version sent with every request.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// What the bare "claude" routing id runs on the Anthropic API.
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5-5";

fn is_anthropic(url: &str) -> bool {
    url.ends_with("/v1/messages")
}

/// A failed model call: HTTP status when there was a response, and a short redacted message.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: Option<u16>,
    pub message: String,
    /// Seconds from a `retry-after` header.
    pub retry_after: Option<u64>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(s) => write!(f, "API returned {s}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// The provider's own error message out of an error body (OpenAI and Anthropic shapes), redacted.
pub(crate) fn error_message(body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let msg = parsed.as_ref().and_then(|v| {
        let err = &v["error"];
        let kind = err["type"].as_str().or_else(|| err["code"].as_str());
        let text = err["message"].as_str().or_else(|| err.as_str());
        match (kind, text) {
            (Some(k), Some(t)) => Some(format!("{k}: {t}")),
            (None, Some(t)) | (Some(t), None) => Some(t.to_string()),
            _ => None,
        }
    });
    crate::profiles::classify::redact_detail(&msg.unwrap_or_else(|| body.to_string()), 300)
}

/// Turns the OpenAI-style message list into an Anthropic Messages body: system prompts move to
/// the top-level `system`, same-role neighbours are merged and the first message is the user's.
fn anthropic_body(model: &str, messages: &[Value], max_tokens: i64) -> Value {
    let mut system = Vec::new();
    let mut turns: Vec<(String, String)> = Vec::new();
    for m in messages {
        let role = m["role"].as_str().unwrap_or("user");
        let content = m["content"].as_str().unwrap_or_default().to_string();
        if role == "system" {
            system.push(content);
            continue;
        }
        let role = if role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        match turns.last_mut() {
            Some((r, c)) if r == role => {
                c.push_str("\n\n");
                c.push_str(&content);
            }
            _ => turns.push((role.to_string(), content)),
        }
    }
    if turns.first().is_some_and(|(r, _)| r == "assistant") {
        turns.insert(0, ("user".into(), "(conversation so far)".into()));
    }
    let model = if model == "claude" {
        DEFAULT_CLAUDE_MODEL
    } else {
        model
    };
    json!({
        "model": model,
        "max_tokens": max_tokens.max(1),
        "system": system.join("\n\n"),
        "messages": turns.into_iter().map(|(role, content)| json!({"role": role, "content": content})).collect::<Vec<_>>(),
    })
}

/// The reply text of a non-streamed call, OpenAI or Anthropic shaped.
fn reply_text(reply: &Value) -> Option<String> {
    let anthropic_text = reply["content"].as_array().map(|blocks| {
        blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    });
    reply["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .or(anthropic_text)
}

/// Tokens a reply used, from OpenAI's `total_tokens` or Anthropic's input + output.
fn usage_tokens(usage: &Value) -> Option<u64> {
    usage["total_tokens"]
        .as_u64()
        .or_else(|| Some(usage["input_tokens"].as_u64()? + usage["output_tokens"].as_u64().unwrap_or(0)))
}

/// Reads a streamed reply (server-sent events, OpenAI or Anthropic shaped), sending each piece
/// of text as it arrives. A server that answers with one JSON body instead is read whole. Returns
/// the full text and tokens used; a stream cut off midway keeps what arrived, with a note.
async fn stream_reply(
    mut resp: reqwest::Response,
    anthropic: bool,
    event_tx: &mpsc::Sender<RuntimeEvent>,
) -> (String, usize) {
    let is_sse = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"));
    if !is_sse {
        let reply = resp.json::<Value>().await.unwrap_or(Value::Null);
        let text = reply_text(&reply).unwrap_or_else(|| "No response content".into());
        let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
        return (text, usage_tokens(&reply["usage"]).unwrap_or(45) as usize);
    }
    let mut text = String::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut total_tokens: Option<u64> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut failure: Option<String> = None;
    'read: loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(_) => {
                failure = Some("the stream was cut off".into());
                break;
            }
        };
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                break 'read;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if event.get("error").is_some_and(|e| !e.is_null()) {
                failure = Some(error_message(data));
                break 'read;
            }
            let piece = if anthropic {
                match event["type"].as_str() {
                    Some("message_start") => {
                        input_tokens = event["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
                        None
                    }
                    Some("message_delta") => {
                        output_tokens = event["usage"]["output_tokens"].as_u64().unwrap_or(output_tokens);
                        None
                    }
                    Some("content_block_delta") => event["delta"]["text"].as_str(),
                    _ => None,
                }
            } else {
                if let Some(t) = usage_tokens(&event["usage"]) {
                    total_tokens = Some(t);
                }
                event["choices"][0]["delta"]["content"].as_str()
            };
            if let Some(piece) = piece.filter(|p| !p.is_empty()) {
                text.push_str(piece);
                let _ = event_tx.send(RuntimeEvent::TextDelta(piece.to_string())).await;
            }
        }
    }
    if let Some(why) = failure {
        let note = format!("{}⚠️ The reply stopped early: {why}", if text.is_empty() { "" } else { "\n\n" });
        text.push_str(&note);
        let _ = event_tx.send(RuntimeEvent::TextDelta(note)).await;
    } else if text.is_empty() {
        text = "No response content".into();
        let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
    }
    let tokens = total_tokens.unwrap_or(input_tokens + output_tokens);
    (text, if tokens == 0 { 45 } else { tokens as usize })
}

struct ChatRequest<'a> {
    url: &'a str,
    api_key: Option<&'a str>,
    model: &'a str,
    temperature: f64,
    max_tokens: i64,
    /// Model calls allowed this turn; each round of MCP tool calls costs one.
    max_calls: usize,
    /// Stream the reply as it is written (rounds without tools only).
    stream: bool,
}

#[derive(Clone, Debug)]
pub struct ApiDriver {
    client: Client,
    api_key: Option<String>,
    endpoint: Option<String>,
    /// Whether a missing key may come from the provider's env var (not for profiles).
    env_keys: bool,
    /// A Custom API account's base URL: no key needed, replies streamed, and a server that
    /// doesn't answer is named in the error.
    custom_base: Option<String>,
}

impl ApiDriver {
    pub fn new(api_key: Option<String>, endpoint: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            endpoint,
            env_keys: true,
            custom_base: None,
        }
    }

    /// A Custom API account's driver: turns go to `chat_url` (see [`super::custom::chat_url`])
    /// with the account's own key, or none; never a host key.
    pub fn for_custom(api_key: Option<String>, chat_url: String, base: String) -> Self {
        Self {
            env_keys: false,
            custom_base: Some(base),
            ..Self::new(api_key, Some(chat_url))
        }
    }

    /// An `api_key` profile's driver: only ever its own key, never the host's env key.
    pub fn for_profile(api_key: Option<String>) -> Self {
        Self {
            env_keys: false,
            ..Self::new(api_key, None)
        }
    }

    fn key_or_env(&self, var: &str) -> Option<String> {
        self.api_key.clone().or_else(|| {
            if self.env_keys {
                std::env::var(var).ok()
            } else {
                None
            }
        })
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
            let key = self.key_or_env("ANTHROPIC_API_KEY");
            ("https://api.anthropic.com/v1/messages".to_string(), key)
        } else if model.contains("grok") {
            let key = self.key_or_env("XAI_API_KEY");
            ("https://api.x.ai/v1/chat/completions".to_string(), key)
        } else {
            let key = self.key_or_env("OPENAI_API_KEY");
            ("https://api.openai.com/v1/chat/completions".to_string(), key)
        }
    }
}

impl ApiDriver {
    /// Sends one model call; `Err` for no answer or an error status, with the provider's message.
    async fn send_chat(&self, request: &ChatRequest<'_>, body: &Value) -> Result<reqwest::Response, ApiError> {
        let mut req = self
            .client
            .post(request.url)
            .json(body)
            .timeout(Duration::from_secs(300));
        if let Some(key) = request.api_key {
            req = if is_anthropic(request.url) {
                req.header("x-api-key", key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
            } else {
                req.header("Authorization", format!("Bearer {}", key))
            };
        }
        let resp = req.send().await.map_err(|e| ApiError {
            status: None,
            message: match (&self.custom_base, e.is_timeout()) {
                (_, true) => "request timed out".into(),
                (Some(base), false) => super::custom::not_answering(base),
                (None, false) => "connection failed".into(),
            },
            retry_after: None,
        })?;
        let status = resp.status();
        if !status.is_success() {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError {
                status: Some(status.as_u16()),
                message: error_message(&body),
                retry_after,
            });
        }
        Ok(resp)
    }

    async fn post_chat(&self, request: &ChatRequest<'_>, body: &Value) -> Result<Value, ApiError> {
        let resp = self.send_chat(request, body).await?;
        resp.json::<Value>().await.map_err(|_| ApiError {
            status: None,
            message: "invalid response from the model API".into(),
            retry_after: None,
        })
    }

    /// `post_chat`, retrying overloads, rate limits and timeouts twice before giving up.
    async fn post_with_retry(
        &self,
        request: &ChatRequest<'_>,
        body: &Value,
    ) -> Result<Value, ApiError> {
        self.with_retry(|| self.post_chat(request, body)).await
    }

    async fn with_retry<T, F, Fut>(&self, call: F) -> Result<T, ApiError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, ApiError>>,
    {
        let mut attempt = 0;
        loop {
            match call().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let class = crate::profiles::classify(e.status, &e.message, chrono::Utc::now())
                        .map(|c| c.0);
                    if attempt >= 2 || class != Some(crate::profiles::FailClass::Transient) {
                        return Err(e);
                    }
                    let wait = e
                        .retry_after
                        .filter(|s| *s <= 30)
                        .unwrap_or(if attempt == 0 { 1 } else { 4 });
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Calls the model, running any MCP tool calls it asks for and calling it again with the
    /// results, up to `max_calls` model calls. Returns the final text and tokens used, or the
    /// error when the very first call fails.
    async fn chat_loop(
        &self,
        request: &ChatRequest<'_>,
        mut messages: Vec<Value>,
        session: Option<&McpToolSession>,
        caller: &Caller,
        event_tx: &mpsc::Sender<RuntimeEvent>,
    ) -> Result<(String, usize, bool), ApiError> {
        let tools = session.map(|s| s.openai_tools()).unwrap_or_default();
        let max_calls = request.max_calls.max(1);
        let mut tokens = 0usize;
        if request.stream && tools.is_empty() {
            let mut body = if is_anthropic(request.url) {
                anthropic_body(request.model, &messages, request.max_tokens)
            } else {
                json!({
                    "model": request.model,
                    "messages": messages,
                    "temperature": request.temperature,
                    "max_tokens": request.max_tokens,
                })
            };
            body["stream"] = json!(true);
            let resp = self.with_retry(|| self.send_chat(request, &body)).await?;
            let (text, tokens) = stream_reply(resp, is_anthropic(request.url), event_tx).await;
            return Ok((text, tokens, true));
        }
        for round in 0..max_calls {
            let body = if is_anthropic(request.url) {
                anthropic_body(request.model, &messages, request.max_tokens)
            } else {
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
                body
            };
            let reply = match self.post_with_retry(request, &body).await {
                Ok(v) => v,
                Err(e) if round == 0 => return Err(e),
                Err(e) => {
                    return Ok((
                        format!("⚠️ The model request failed after running tools: {e}"),
                        tokens,
                        false,
                    ))
                }
            };
            tokens += usage_tokens(&reply["usage"]).unwrap_or(45) as usize;
            let message = &reply["choices"][0]["message"];
            let content = reply_text(&reply);
            let calls = message["tool_calls"].as_array().filter(|c| !c.is_empty());

            let (Some(calls), Some(session)) = (calls, session) else {
                return Ok((
                    content.unwrap_or_else(|| "No response content".into()),
                    tokens,
                    false,
                ));
            };
            if round + 1 == max_calls {
                let note = format!("(Stopped: reached the limit of {max_calls} model calls for MCP tools this turn.)");
                let text = match content.filter(|c| !c.trim().is_empty()) {
                    Some(c) => format!("{c}\n\n{note}"),
                    None => note,
                };
                return Ok((text, tokens, false));
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
                if call["id"].as_str().is_none_or(str::is_empty) {
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
        Err(ApiError {
            status: None,
            message: "no model calls were allowed".into(),
            retry_after: None,
        })
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
        let custom = self.custom_base.is_some();
        if !self.env_keys && api_key.is_none() && !custom {
            return Ok(TurnOutput::refused(crate::profiles::ProviderFailure {
                class: crate::profiles::FailClass::Auth,
                retry_at: None,
                detail: "this profile has no API key".into(),
            }));
        }

        // If target is a real live API endpoint with API key or local Ollama, call HTTP streaming
        if api_key.is_some() || is_ollama(target_model) || custom {
            // An API turn has no shell: the browser is reachable only through the tools, which the
            // Anthropic Messages format doesn't get
            let browser = if input.has_self_tools() && !url.ends_with("/v1/messages") {
                super::BrowserAccess::Tools
            } else {
                super::BrowserAccess::Unavailable
            };
            let system_prompt = input.format_system_prompt_with(browser);

            let mut messages = vec![json!({"role": "system", "content": system_prompt})];
            for msg in &input.history {
                let (role, content) = match msg.sender_role.as_str() {
                    "kwant" if msg.sender_id == input.kwant.name => ("assistant", msg.text.clone()),
                    // Teammates' replies are context from other speakers, not this kwant's own turns
                    "kwant" => ("user", super::peer_message(&msg.sender_id, &msg.text)),
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

            if custom && input.model.is_none() {
                return Err(DriverError::Execution(
                    "This Custom API account has no model to run: enter a model id for it under Space settings → Accounts, or pin one on the bot (\"custom:<model>\").".into(),
                ));
            }
            let model = match api_model_name(target_model, input.model.as_deref(), &url, api_key.as_deref(), effort).await {
                Ok(m) => m,
                Err(why) => return Err(DriverError::Execution(why)),
            };
            // The Anthropic Messages API has a different tool format; it is not given MCP tools yet
            let mcp = input.mcp.as_ref().filter(|_| !url.ends_with("/v1/messages"));
            // kwnt's own continuity tools are always attached; only the user's servers are worth a notice
            let has_user_servers = input
                .mcp
                .as_ref()
                .is_some_and(|c| c.servers.iter().any(|s| s.name != crate::mcp::SELF_SERVER_NAME));
            if has_user_servers && mcp.is_none() {
                let _ = event_tx
                    .send(RuntimeEvent::ThinkingDelta("MCP tools aren't offered over the Anthropic Messages API; running without them.".into()))
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
                stream: custom,
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
            let (text, tokens, streamed) = match outcome {
                Ok(done) => done,
                Err(e) => {
                    let now = chrono::Utc::now();
                    match crate::profiles::classify(e.status, &e.message, now) {
                        Some((class, retry_at)) if class.fails_over() => {
                            tracing::warn!(status = ?e.status, class = class.as_str(), "model API refused the turn");
                            return Ok(TurnOutput::refused(crate::profiles::ProviderFailure {
                                class,
                                retry_at,
                                detail: e.message.clone(),
                            }));
                        }
                        _ => {
                            tracing::warn!(status = ?e.status, "model API call failed");
                            (format!("⚠️ The model request failed: {e}"), 0, false)
                        }
                    }
                }
            };
            {
                // A streamed reply was sent piece by piece already
                if !streamed {
                    let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
                }
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens,
                        duration_ms: start.elapsed().as_millis() as u64,
                        kwant_id: None,
                        profile_id: None,
                    })
                    .await;
                return Ok(TurnOutput {
                    text,
                    reasoning: Some(format!("Model: {} [Effort: {}]", model, effort)),
                    tokens_used: tokens,
                    model: Some(model.clone()),
                    ..Default::default()
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
                kwant_id: None,
                profile_id: None,
            })
            .await;

        Ok(TurnOutput {
            text: response_text,
            reasoning: Some(format!("Model: {} (Unauthenticated)", target_model)),
            tokens_used: tokens,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

    async fn mock(status: u16, reply: Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let app = Router::new()
            .route(
                "/v1/messages",
                post(
                    |State((seen, status, reply)): State<(Seen, u16, Value)>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        seen.lock().unwrap().push((headers, body));
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            Json(reply),
                        )
                    },
                ),
            )
            .with_state((seen.clone(), status, reply));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    fn request<'a>(url: &'a str, key: &'a str) -> ChatRequest<'a> {
        ChatRequest {
            url,
            api_key: Some(key),
            model: "claude",
            temperature: 0.7,
            max_tokens: 100,
            max_calls: 1,
            stream: false,
        }
    }

    #[tokio::test]
    async fn anthropic_calls_use_x_api_key_and_version() {
        let reply = json!({"content": [{"type": "text", "text": "Hi"}, {"type": "text", "text": " there"}], "usage": {"input_tokens": 5, "output_tokens": 2}});
        let (url, seen) = mock(200, reply).await;
        let driver = ApiDriver::new(Some("sk-ant-test-key".into()), Some(url.clone()));
        let messages = vec![
            json!({"role": "system", "content": "be brief"}),
            json!({"role": "assistant", "content": "earlier"}),
            json!({"role": "user", "content": "a"}),
            json!({"role": "user", "content": "b"}),
        ];
        let (tx, _rx) = mpsc::channel(8);
        let caller = Caller {
            kwant_id: "k".into(),
            kwant_name: "K".into(),
            turn_id: "t".into(),
        };
        let (text, tokens, streamed) = driver
            .chat_loop(
                &request(&url, "sk-ant-test-key"),
                messages,
                None,
                &caller,
                &tx,
            )
            .await
            .unwrap();
        assert_eq!(text, "Hi there");
        assert_eq!(tokens, 7);
        assert!(!streamed, "a request without stream is read whole");

        let (headers, body) = seen.lock().unwrap()[0].clone();
        assert_eq!(headers["x-api-key"], "sk-ant-test-key");
        assert_eq!(headers["anthropic-version"], ANTHROPIC_VERSION);
        assert!(headers.get("authorization").is_none());
        assert_eq!(body["model"], DEFAULT_CLAUDE_MODEL);
        assert_eq!(body["system"], "be brief");
        assert!(body.get("stream").is_none(), "existing API-key turns are not streamed");
        assert!(body.get("temperature").is_none());
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(body["messages"][2]["content"], "a\n\nb");
    }

    #[tokio::test]
    async fn quota_errors_are_returned_redacted_without_retry() {
        let err = json!({"type": "error", "error": {"type": "billing_error", "message": "Your credit balance is too low for key sk-ant-api03-abcdefghijklmnop0123456789"}});
        let (url, seen) = mock(400, err).await;
        let driver = ApiDriver::new(Some("k".repeat(20)), Some(url.clone()));
        let e = driver
            .post_with_retry(&request(&url, "kkkkkkkkkkkkkkkkkkkk"), &json!({}))
            .await
            .unwrap_err();
        assert_eq!(e.status, Some(400));
        assert!(e.message.contains("credit balance is too low"), "{e}");
        assert!(!e.message.contains("abcdefghij"), "{e}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        let class = crate::profiles::classify(e.status, &e.message, chrono::Utc::now())
            .unwrap()
            .0;
        assert_eq!(class, crate::profiles::FailClass::Quota);
    }

    #[test]
    fn profile_drivers_never_use_env_keys() {
        std::env::set_var("XAI_API_KEY", "xai-host-key");
        let (_, key) = ApiDriver::for_profile(None).resolve_endpoint_and_key("grok-4");
        assert!(key.is_none());
        let (_, key) =
            ApiDriver::for_profile(Some("xai-profile".into())).resolve_endpoint_and_key("grok-4");
        assert_eq!(key.as_deref(), Some("xai-profile"));
        let (_, key) = ApiDriver::new(None, None).resolve_endpoint_and_key("grok-4");
        assert_eq!(key.as_deref(), Some("xai-host-key"));
    }
}
