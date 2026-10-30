//! A small MCP client: JSON-RPC over a child process's stdio, or over streamable HTTP.
//! Supports what kwnt needs: initialize, tools/list, tools/call, timeouts and clean shutdown.

use crate::db::models::McpServer;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Variables a stdio server inherits from kwnt; everything else (API keys…) stays private.
const INHERITED_ENV: [&str; 5] = ["PATH", "HOME", "USER", "LANG", "TMPDIR"];
const STDERR_TAIL_BYTES: usize = 4096;
const MAX_TOOL_PAGES: usize = 20;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("could not start server: {0}")]
    Spawn(String),
    #[error("timed out after {0}s waiting for {1}")]
    Timeout(u64, String),
    #[error("server closed the connection")]
    Closed,
    #[error("server error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("transport error: {0}")]
    Transport(String),
    /// HTTP 401: the server wants a (new) access token. Keeps its `WWW-Authenticate` header.
    #[error("transport error: HTTP 401 Unauthorized (sign in to this server from the Tools screen)")]
    Unauthorized { www_authenticate: Option<String> },
    #[error("invalid response: {0}")]
    Protocol(String),
    /// The server failed before the handshake completed; keeps what it wrote to stderr.
    #[error("{message}")]
    Startup { message: String, stderr_tail: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    /// The server marks it read-only (`readOnlyHint: true`, not `destructiveHint: true`). The
    /// server's own claim: kwnt can't check it.
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallResult {
    pub text: String,
    pub is_error: bool,
}

type Pending = Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>;

struct StdioTransport {
    child: Mutex<Option<Child>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    stderr_tail: Arc<StdMutex<String>>,
    tasks: Vec<JoinHandle<()>>,
}

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: reqwest::header::HeaderMap,
    session_id: StdMutex<Option<String>>,
    protocol_version: StdMutex<Option<String>>,
}

enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

pub struct McpClient {
    pub server_name: String,
    pub server_info: Value,
    timeout: Duration,
    next_id: AtomicI64,
    transport: Transport,
}

async fn write_line(stdin: &Mutex<Option<ChildStdin>>, msg: &Value) -> Result<(), McpError> {
    let mut guard = stdin.lock().await;
    let pipe = guard.as_mut().ok_or(McpError::Closed)?;
    let mut line = serde_json::to_vec(msg).map_err(|e| McpError::Protocol(e.to_string()))?;
    line.push(b'\n');
    pipe.write_all(&line).await.map_err(|_| McpError::Closed)?;
    pipe.flush().await.map_err(|_| McpError::Closed)
}

fn response_id(msg: &Value) -> Option<i64> {
    if msg.get("method").is_some() || (msg.get("result").is_none() && msg.get("error").is_none()) {
        return None;
    }
    msg.get("id")?.as_i64()
}

/// Text an agent can read from a `tools/call` result.
fn result_text(result: &Value) -> String {
    let mut parts = Vec::new();
    for item in result["content"].as_array().into_iter().flatten() {
        match item["type"].as_str().unwrap_or("") {
            "text" => parts.push(item["text"].as_str().unwrap_or("").to_string()),
            "image" => parts.push(format!("[image: {}]", item["mimeType"].as_str().unwrap_or("?"))),
            "audio" => parts.push(format!("[audio: {}]", item["mimeType"].as_str().unwrap_or("?"))),
            "resource" => parts.push(
                item["resource"]["text"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("[resource: {}]", item["resource"]["uri"].as_str().unwrap_or("?"))),
            ),
            "resource_link" => parts.push(format!("[resource link: {}]", item["uri"].as_str().unwrap_or("?"))),
            other => parts.push(format!("[{other} content]")),
        }
    }
    if parts.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            return structured.to_string();
        }
    }
    parts.join("\n")
}

impl McpClient {
    /// Starts or connects to `server` and completes the MCP handshake. Stdio servers run in `cwd`.
    pub async fn connect(server: &McpServer, cwd: Option<&Path>) -> Result<Self, McpError> {
        let timeout = Duration::from_secs(server.timeout_secs.clamp(1, 600) as u64);
        let transport = match server.transport.as_str() {
            "stdio" => Transport::Stdio(Self::spawn_stdio(server, cwd)?),
            "http" => Transport::Http(Self::http_transport(server)?),
            other => return Err(McpError::Spawn(format!("unknown transport '{other}'"))),
        };
        let mut client = Self {
            server_name: server.name.clone(),
            server_info: Value::Null,
            timeout,
            next_id: AtomicI64::new(1),
            transport,
        };
        match client.handshake().await {
            Ok(()) => Ok(client),
            Err(e @ McpError::Unauthorized { .. }) => Err(e),
            Err(e) => {
                client.shutdown().await;
                Err(McpError::Startup { message: e.to_string(), stderr_tail: client.stderr_tail() })
            }
        }
    }

    fn spawn_stdio(server: &McpServer, cwd: Option<&Path>) -> Result<StdioTransport, McpError> {
        // fastbrowse keeps its rules even when its row was saved before them
        let checked;
        let server = if super::fastbrowse::is_fastbrowse(server) {
            checked = super::fastbrowse::check_before_start(server).map_err(McpError::Spawn)?;
            &checked
        } else {
            server
        };
        let program = server.command.as_deref().ok_or_else(|| McpError::Spawn("no command".into()))?;
        let mut cmd = Command::new(program);
        cmd.args(&server.args).env_clear();
        for key in INHERITED_ENV {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        cmd.envs(&server.env);
        if let Some(dir) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| McpError::Spawn(format!("{program}: {e}")))?;

        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().ok_or_else(|| McpError::Spawn("no stdout".into()))?;
        let mut stderr = child.stderr.take().ok_or_else(|| McpError::Spawn("no stderr".into()))?;
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let stderr_tail = Arc::new(StdMutex::new(String::new()));

        let reader = {
            let (pending, closed, stdin) = (pending.clone(), closed.clone(), stdin.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                        tracing::debug!("ignoring non-JSON line from MCP server");
                        continue;
                    };
                    if let Some(id) = response_id(&msg) {
                        if let Some(tx) = pending.lock().ok().and_then(|mut p| p.remove(&id)) {
                            let _ = tx.send(msg);
                        }
                    } else if let (Some(method), Some(id)) = (msg["method"].as_str(), msg.get("id")) {
                        // Requests from the server: answer ping, decline the rest
                        let reply = if method == "ping" {
                            json!({"jsonrpc": "2.0", "id": id, "result": {}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not supported by kwnt"}})
                        };
                        let _ = write_line(&stdin, &reply).await;
                    }
                }
                closed.store(true, Ordering::SeqCst);
                // Dropping the senders wakes every waiting request with `Closed`
                if let Ok(mut p) = pending.lock() {
                    p.clear();
                }
            })
        };
        let stderr_task = {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut t) = tail.lock() {
                        t.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if t.len() > STDERR_TAIL_BYTES {
                            let mut cut = t.len() - STDERR_TAIL_BYTES;
                            while !t.is_char_boundary(cut) {
                                cut += 1;
                            }
                            t.drain(..cut);
                        }
                    }
                }
            })
        };

        Ok(StdioTransport {
            child: Mutex::new(Some(child)),
            stdin,
            pending,
            closed,
            stderr_tail,
            tasks: vec![reader, stderr_task],
        })
    }

    fn http_transport(server: &McpServer) -> Result<HttpTransport, McpError> {
        let url = server.url.clone().ok_or_else(|| McpError::Spawn("no url".into()))?;
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in &server.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|_| McpError::Spawn(format!("invalid header name '{k}'")))?;
            let mut value = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| McpError::Spawn(format!("invalid value for header '{k}'")))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| McpError::Spawn(e.to_string()))?;
        Ok(HttpTransport {
            client,
            url,
            headers,
            session_id: StdMutex::new(None),
            protocol_version: StdMutex::new(None),
        })
    }

    async fn handshake(&mut self) -> Result<(), McpError> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "kwnt", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        self.server_info = result.get("serverInfo").cloned().unwrap_or(Value::Null);
        if let Transport::Http(ref http) = self.transport {
            let version = result["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION).to_string();
            *http.protocol_version.lock().unwrap() = Some(version);
        }
        self.notify("notifications/initialized", json!({})).await
    }

    /// The last few KB the server wrote to stderr (stdio only).
    pub fn stderr_tail(&self) -> String {
        match &self.transport {
            Transport::Stdio(s) => s.stderr_tail.lock().map(|t| t.clone()).unwrap_or_default(),
            Transport::Http(_) => String::new(),
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            let page = result["tools"].as_array().ok_or_else(|| McpError::Protocol("tools/list without tools".into()))?;
            for t in page {
                let Some(name) = t["name"].as_str() else { continue };
                tools.push(McpTool {
                    name: name.to_string(),
                    description: t["description"].as_str().map(str::to_string),
                    input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                    read_only: t["annotations"]["readOnlyHint"].as_bool() == Some(true)
                        && t["annotations"]["destructiveHint"].as_bool() != Some(true),
                });
            }
            cursor = result["nextCursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult, McpError> {
        let args = if arguments.is_object() { arguments } else { json!({}) };
        let result = self.request("tools/call", json!({"name": name, "arguments": args})).await?;
        Ok(ToolCallResult {
            text: result_text(&result),
            is_error: result["isError"].as_bool().unwrap_or(false),
        })
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = match &self.transport {
            Transport::Stdio(s) => self.stdio_request(s, id, method, &msg).await?,
            Transport::Http(h) => {
                tokio::time::timeout(self.timeout, self.http_post(h, &msg, Some(id)))
                    .await
                    .map_err(|_| McpError::Timeout(self.timeout.as_secs(), method.to_string()))??
                    .ok_or_else(|| McpError::Protocol(format!("no response to {method}")))?
            }
        };
        if let Some(err) = response.get("error") {
            return Err(McpError::Rpc {
                code: err["code"].as_i64().unwrap_or(0),
                message: err["message"].as_str().unwrap_or("unknown error").to_string(),
            });
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn stdio_request(&self, s: &StdioTransport, id: i64, method: &str, msg: &Value) -> Result<Value, McpError> {
        if s.closed.load(Ordering::SeqCst) {
            return Err(McpError::Closed);
        }
        let (tx, rx) = oneshot::channel();
        s.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = write_line(&s.stdin, msg).await {
            s.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(McpError::Closed),
            Err(_) => {
                s.pending.lock().unwrap().remove(&id);
                let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                    "params": {"requestId": id, "reason": "timeout"}});
                let _ = write_line(&s.stdin, &cancel).await;
                Err(McpError::Timeout(self.timeout.as_secs(), method.to_string()))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        match &self.transport {
            Transport::Stdio(s) => write_line(&s.stdin, &msg).await,
            Transport::Http(h) => tokio::time::timeout(self.timeout, self.http_post(h, &msg, None))
                .await
                .map_err(|_| McpError::Timeout(self.timeout.as_secs(), method.to_string()))?
                .map(|_| ()),
        }
    }

    /// POSTs one message. For requests, returns the matching response from a JSON or SSE body.
    async fn http_post(&self, h: &HttpTransport, msg: &Value, id: Option<i64>) -> Result<Option<Value>, McpError> {
        let mut req = h
            .client
            .post(&h.url)
            .headers(h.headers.clone())
            .header("Accept", "application/json, text/event-stream")
            .json(msg);
        if let Some(sid) = h.session_id.lock().unwrap().clone() {
            req = req.header("Mcp-Session-Id", sid);
        }
        if let Some(v) = h.protocol_version.lock().unwrap().clone() {
            req = req.header("MCP-Protocol-Version", v);
        }
        let resp = req.send().await.map_err(|e| McpError::Transport(e.without_url().to_string()))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let www_authenticate = resp
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            return Err(McpError::Unauthorized { www_authenticate });
        }
        if !status.is_success() {
            return Err(McpError::Transport(format!("HTTP {status}")));
        }
        if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *h.session_id.lock().unwrap() = Some(sid.to_string());
        }
        let Some(id) = id else {
            return Ok(None);
        };
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if content_type.starts_with("text/event-stream") {
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| McpError::Transport(e.without_url().to_string()))?;
                buf.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
                while let Some(end) = buf.find("\n\n") {
                    let event: String = buf.drain(..end + 2).collect();
                    let data = event
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:"))
                        .map(|d| d.strip_prefix(' ').unwrap_or(d))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if let Ok(msg) = serde_json::from_str::<Value>(&data) {
                        if response_id(&msg) == Some(id) {
                            return Ok(Some(msg));
                        }
                    }
                }
            }
            return Err(McpError::Protocol("event stream ended before the response".into()));
        }

        let body: Value = resp.json().await.map_err(|e| McpError::Protocol(e.without_url().to_string()))?;
        let found = match body {
            Value::Array(items) => items.into_iter().find(|m| response_id(m) == Some(id)),
            single => (response_id(&single) == Some(id)).then_some(single),
        };
        found.map(Some).ok_or_else(|| McpError::Protocol("response id does not match".into()))
    }

    /// Ends the session: closes stdin and waits briefly, then kills the process (stdio), or
    /// deletes the session (HTTP). Always safe to call.
    pub async fn shutdown(&mut self) {
        match &mut self.transport {
            Transport::Stdio(s) => {
                s.stdin.lock().await.take();
                if let Some(mut child) = s.child.lock().await.take() {
                    if tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                    }
                }
                // Let the readers drain what the process wrote before it exited
                for mut task in s.tasks.drain(..) {
                    if tokio::time::timeout(Duration::from_millis(500), &mut task).await.is_err() {
                        task.abort();
                    }
                }
            }
            Transport::Http(h) => {
                let sid = h.session_id.lock().unwrap().take();
                if let Some(sid) = sid {
                    let req = h.client.delete(&h.url).headers(h.headers.clone()).header("Mcp-Session-Id", sid);
                    let _ = tokio::time::timeout(Duration::from_secs(2), req.send()).await;
                }
            }
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // `kill_on_drop` stops the process; also stop the reader tasks
        for task in &self.tasks {
            task.abort();
        }
    }
}
