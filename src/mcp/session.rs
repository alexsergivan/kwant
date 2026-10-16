//! One API-agent turn's MCP connections: offers the servers' tools to the model and runs the
//! calls it makes, each one approved through the turn's [`ToolApprover`] and recorded.

use super::{
    client::{McpClient, McpTool},
    namespaced_tool_name, redact, truncate_text, McpTurnContext, ToolApprovalRequest, RECORD_RESULT_MAX_CHARS,
};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

/// OpenAI-compatible APIs reject function names outside `^[a-zA-Z0-9_-]{1,64}$`.
fn valid_function_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

struct OfferedTool {
    name: String,
    server_index: usize,
    tool: McpTool,
}

/// Who is asking, for approval cards.
#[derive(Debug, Clone)]
pub struct Caller {
    pub kwant_id: String,
    pub kwant_name: String,
    pub turn_id: String,
}

pub struct McpToolSession {
    ctx: McpTurnContext,
    clients: Vec<(String, McpClient)>,
    tools: Vec<OfferedTool>,
    secrets: Vec<String>,
}

/// The outcome of one call, already redacted; `text` goes back to the model.
#[derive(Debug, Clone)]
pub struct CallOutcome {
    pub text: String,
    pub status: &'static str,
    pub record: Value,
}

impl McpToolSession {
    /// Connects to every server and lists its tools. Servers that fail are skipped; their
    /// (redacted) errors are returned so the user can be told.
    pub async fn open(ctx: &McpTurnContext, cwd: Option<&Path>) -> (Self, Vec<String>) {
        let secrets = ctx.secret_values();
        let mut session = Self { ctx: ctx.clone(), clients: Vec::new(), tools: Vec::new(), secrets };
        let mut problems = Vec::new();
        let connections = ctx.servers.iter().map(|s| async move {
            let client = McpClient::connect(s, cwd).await?;
            match client.list_tools().await {
                Ok(tools) => Ok((client, tools)),
                Err(e) => {
                    let mut client = client;
                    client.shutdown().await;
                    Err(e)
                }
            }
        });
        let results = futures::future::join_all(connections).await;
        for (server, result) in ctx.servers.iter().zip(results) {
            match result {
                Ok((client, tools)) => {
                    let index = session.clients.len();
                    for tool in tools {
                        let name = namespaced_tool_name(&server.name, &tool.name);
                        if valid_function_name(&name) {
                            session.tools.push(OfferedTool { name, server_index: index, tool });
                        } else {
                            tracing::warn!(server = %server.name, tool = %tool.name, "MCP tool name not usable as a function name; skipped");
                        }
                    }
                    session.clients.push((server.name.clone(), client));
                }
                Err(e) => problems.push(format!("{}: {}", server.name, redact(&e.to_string(), &session.secrets))),
            }
        }
        (session, problems)
    }

    /// The `tools` field for an OpenAI-compatible chat completion request.
    pub fn openai_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                let mut parameters = t.tool.input_schema.clone();
                if !parameters.is_object() {
                    parameters = json!({"type": "object", "properties": {}});
                }
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.tool.description.clone().unwrap_or_default(),
                        "parameters": parameters,
                    }
                })
            })
            .collect()
    }

    /// Runs one model-requested call: approval first, then the MCP call. The record is also
    /// appended to the turn's `tool_calls`.
    pub async fn call(&self, caller: &Caller, call_id: &str, name: &str, raw_arguments: &str) -> CallOutcome {
        let start = Instant::now();
        let arguments: Value = if raw_arguments.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(raw_arguments) {
                Ok(v) => v,
                Err(_) => Value::String(raw_arguments.to_string()),
            }
        };
        let offered = self.tools.iter().find(|t| t.name == name);
        let (status, text) = match offered {
            None => ("error", format!("Unknown tool {name}.")),
            Some(_) if !arguments.is_object() => ("error", "Tool arguments must be a JSON object.".to_string()),
            Some(t) => {
                let (server, client) = &self.clients[t.server_index];
                let approved = self
                    .ctx
                    .approver
                    .approve(ToolApprovalRequest {
                        kwant_id: caller.kwant_id.clone(),
                        kwant_name: caller.kwant_name.clone(),
                        turn_id: caller.turn_id.clone(),
                        tool_name: name.to_string(),
                        server: server.clone(),
                        tool: t.tool.name.clone(),
                        arguments: arguments.clone(),
                    })
                    .await;
                if !approved {
                    ("denied", "The user denied this tool call.".to_string())
                } else {
                    match client.call_tool(&t.tool.name, arguments.clone()).await {
                        Ok(r) if r.is_error => ("error", r.text),
                        Ok(r) => ("ok", r.text),
                        Err(e) => ("error", format!("Tool call failed: {e}")),
                    }
                }
            }
        };
        let text = redact(&text, &self.secrets);
        let (server, tool) = super::split_tool_name(name).unwrap_or(("", name));
        let mut record = json!({
            "id": call_id,
            "name": name,
            "server": server,
            "tool": tool,
            "arguments": arguments,
            "status": status,
            "duration_ms": start.elapsed().as_millis() as u64,
        });
        if status != "denied" {
            record["result"] = json!(truncate_text(&text, RECORD_RESULT_MAX_CHARS));
        }
        self.ctx.record(record.clone());
        CallOutcome { text, status, record }
    }

    pub async fn shutdown(mut self) {
        for (_, client) in &mut self.clients {
            client.shutdown().await;
        }
    }
}
