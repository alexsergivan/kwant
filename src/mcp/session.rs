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
    /// The name grok sees (see [`grok_tool_names`]).
    grok_name: String,
}

/// How kwnt's gateway names the tools it offers an agent CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayNaming {
    /// `<server>__<tool>` (codex).
    Plain,
    /// `<server>-<tool>` with no `__` (grok rejects `__` inside an MCP tool name).
    Grok,
}

/// grok's name for `<server>`'s `<tool>`: `<server>-<tool>`, every character outside
/// `[A-Za-z0-9_-]` replaced by `_` and runs of `_` collapsed, so there is never a `__`.
fn grok_base_name(server: &str, tool: &str) -> String {
    let mut name = String::with_capacity(server.len() + tool.len() + 1);
    for c in format!("{server}-{tool}").chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '_'
        };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    name
}

/// grok names for `(server, tool)` pairs, in order. Distinct tools whose names map to the same
/// grok name get a `-2`, `-3`, … suffix (first one keeps the plain name), never reusing a name
/// another tool maps to plainly, so every grok name stands for exactly one tool.
pub fn grok_tool_names(tools: &[(&str, &str)]) -> Vec<String> {
    let bases: Vec<String> = tools
        .iter()
        .map(|(server, tool)| grok_base_name(server, tool))
        .collect();
    let mut taken: std::collections::HashSet<String> = bases.iter().cloned().collect();
    let mut seen = std::collections::HashSet::new();
    bases
        .into_iter()
        .map(|base| {
            if seen.insert(base.clone()) {
                return base;
            }
            let name = (2..)
                .map(|n| format!("{base}-{n}"))
                .find(|candidate| !taken.contains(candidate))
                .unwrap();
            taken.insert(name.clone());
            name
        })
        .collect()
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
                            session.tools.push(OfferedTool {
                                name,
                                server_index: index,
                                tool,
                                grok_name: String::new(),
                            });
                        } else {
                            tracing::warn!(server = %server.name, tool = %tool.name, "MCP tool name not usable as a function name; skipped");
                        }
                    }
                    session.clients.push((server.name.clone(), client));
                }
                Err(e) => problems.push(format!("{}: {}", server.name, redact(&e.to_string(), &session.secrets))),
            }
        }
        let pairs: Vec<(&str, &str)> = session
            .tools
            .iter()
            .map(|t| {
                (
                    session.clients[t.server_index].0.as_str(),
                    t.tool.name.as_str(),
                )
            })
            .collect();
        let grok_names = grok_tool_names(&pairs);
        for (tool, grok_name) in session.tools.iter_mut().zip(grok_names) {
            tool.grok_name = grok_name;
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

    /// The `tools/list` result for kwnt's MCP gateway. Tools are named `<server>__<tool>` (or
    /// grok's `<server>-<tool>`), so an agent CLI that prefixes its own server name still shows
    /// which server a tool belongs to.
    pub fn gateway_tools(&self, naming: GatewayNaming) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                let mut schema = t.tool.input_schema.clone();
                if !schema.is_object() {
                    schema = json!({"type": "object", "properties": {}});
                }
                let name = match naming {
                    GatewayNaming::Plain => t.name.strip_prefix("mcp__").unwrap_or(&t.name),
                    GatewayNaming::Grok => &t.grok_name,
                };
                json!({
                    "name": name,
                    "description": t.tool.description.clone().unwrap_or_default(),
                    "inputSchema": schema,
                })
            })
            .collect()
    }

    /// The `mcp__<server>__<tool>` name a gateway tool name stands for; the approval card and
    /// the call record use it. `None` for a grok name the gateway never offered.
    pub fn canonical_name(&self, naming: GatewayNaming, gateway_name: &str) -> Option<String> {
        match naming {
            GatewayNaming::Plain => Some(format!("mcp__{gateway_name}")),
            GatewayNaming::Grok => self
                .tools
                .iter()
                .find(|t| t.grok_name == gateway_name)
                .map(|t| t.name.clone()),
        }
    }

    /// Whether the offered tool `name` (`mcp__<server>__<tool>`) says it is read-only; `None`
    /// for a tool this session doesn't offer.
    pub fn read_only(&self, name: &str) -> Option<bool> {
        self.tools.iter().find(|t| t.name == name).map(|t| t.tool.read_only)
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
                let outcome = self
                    .ctx
                    .approver
                    .decide(ToolApprovalRequest {
                        kwant_id: caller.kwant_id.clone(),
                        kwant_name: caller.kwant_name.clone(),
                        turn_id: caller.turn_id.clone(),
                        tool_name: name.to_string(),
                        server: server.clone(),
                        tool: t.tool.name.clone(),
                        arguments: arguments.clone(),
                        builtin: None,
                        smart: Some(read_only_verdict(t.tool.read_only)),
                        payment: None,
                    })
                    .await;
                if !outcome.allowed() {
                    (outcome.record_status(), outcome.refusal_text().to_string())
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
        if status != "denied" && status != "unanswered" {
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

/// "Ask only when risky" for an MCP tool: read-only tools run, all others ask.
pub fn read_only_verdict(read_only: bool) -> super::SmartVerdict {
    if read_only {
        super::SmartVerdict::Allow("the MCP server marks this tool read-only".into())
    } else {
        super::SmartVerdict::Ask("this MCP tool can change things".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_names_have_no_double_underscore() {
        let names = grok_tool_names(&[
            ("alpha", "alpha_ping"),
            ("gh", "create__issue"),
            ("a-b", "x__y___z_"),
        ]);
        assert_eq!(names, ["alpha-alpha_ping", "gh-create_issue", "a-b-x_y_z_"]);
        assert!(names.iter().all(|n| !n.contains("__")));
    }

    #[test]
    fn grok_name_collisions_get_unique_suffixes() {
        let names = grok_tool_names(&[
            ("a", "b-c"),
            ("a-b", "c"),    // same base as the first
            ("gh", "x__y"),  // `gh-x_y`
            ("gh", "x_y"),   // same base
            ("gh", "x_y-2"), // the suffix the previous one would naively get
            ("a", "b-c"),    // a third `a-b-c`
        ]);
        assert_eq!(
            names,
            ["a-b-c", "a-b-c-2", "gh-x_y", "gh-x_y-3", "gh-x_y-2", "a-b-c-3"]
        );
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert!(names.iter().all(|n| !n.contains("__")));
    }
}
