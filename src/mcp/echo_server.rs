//! A tiny stdio MCP server, run with the hidden `kwnt mcp-echo-server` subcommand. It exists so
//! tests and smoke checks have a real MCP server without Python, Node or network access.
//!
//! Tools: `echo {text}`, `add {a, b}`, `sleep {ms}`, `env {name}` (reads its own environment)
//! and `crash` (exits mid-call). `echo` is marked read-only; `add` claims both read-only and
//! destructive, so it counts as not read-only.
//!
//! `--home-tools` adds Home Assistant-shaped tools (26f): `homeassistant__GetLiveContext`
//! (read-only), `intent__HassTurnOn` and `intent__HassTurnOff` `{name, area, domain,
//! device_class}`, and `homeassistant__schedule_command {command}`; each answers that it did it.
//! `--server-name home-assistant` makes the server say it is Home Assistant's; it says
//! `kwnt-echo` otherwise.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn tools() -> Value {
    let obj = |props: Value, required: Value| json!({"type": "object", "properties": props, "required": required});
    json!([
        {"name": "echo", "description": "Returns the given text unchanged.",
         "inputSchema": obj(json!({"text": {"type": "string"}}), json!(["text"])),
         "annotations": {"readOnlyHint": true}},
        {"name": "add", "description": "Adds two numbers.",
         "inputSchema": obj(json!({"a": {"type": "number"}, "b": {"type": "number"}}), json!(["a", "b"])),
         "annotations": {"readOnlyHint": true, "destructiveHint": true}},
        {"name": "sleep", "description": "Waits for the given milliseconds, then answers.",
         "inputSchema": obj(json!({"ms": {"type": "integer"}}), json!(["ms"]))},
        {"name": "env", "description": "Returns the value of one of this server's environment variables.",
         "inputSchema": obj(json!({"name": {"type": "string"}}), json!(["name"]))},
        {"name": "crash", "description": "Exits the server without answering.",
         "inputSchema": obj(json!({}), json!([]))},
    ])
}

fn home_tools_list() -> Vec<Value> {
    let target = json!({
        "name": {"type": "string"}, "area": {"type": "string"},
        "domain": {"type": "array", "items": {"type": "string"}},
        "device_class": {"type": "array", "items": {"type": "string"}},
    });
    let obj = |props: Value| json!({"type": "object", "properties": props});
    json!([
        {"name": "homeassistant__GetLiveContext", "description": "Reads the current state of the house.",
         "inputSchema": obj(json!({})), "annotations": {"readOnlyHint": true}},
        {"name": "intent__HassTurnOn", "description": "Turns on, opens or locks a device.", "inputSchema": obj(target.clone())},
        {"name": "intent__HassTurnOff", "description": "Turns off, closes or unlocks a device.", "inputSchema": obj(target)},
        {"name": "homeassistant__schedule_command", "description": "Runs a command later.",
         "inputSchema": obj(json!({"command": {"type": "string"}}))},
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

async fn call(name: &str, args: &Value) -> Value {
    match name {
        "homeassistant__GetLiveContext" => text_result("Live context: the lights are on, the front door is locked".into(), false),
        "intent__HassTurnOn" | "intent__HassTurnOff" | "homeassistant__schedule_command" => text_result(format!("done: {name} {args}"), false),
        "echo" => text_result(args["text"].as_str().unwrap_or("").to_string(), false),
        "add" => {
            let sum = args["a"].as_f64().unwrap_or(0.0) + args["b"].as_f64().unwrap_or(0.0);
            text_result(sum.to_string(), false)
        }
        "sleep" => {
            let ms = args["ms"].as_u64().unwrap_or(0);
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            text_result(format!("slept {ms} ms"), false)
        }
        "env" => {
            let var = args["name"].as_str().unwrap_or("");
            match std::env::var(var) {
                Ok(v) => text_result(v, false),
                Err(_) => text_result(format!("{var} is not set"), true),
            }
        }
        "crash" => {
            eprintln!("mcp-echo-server: crashing on request");
            std::process::exit(3);
        }
        other => text_result(format!("unknown tool {other}"), true),
    }
}

/// Serves MCP on stdin/stdout until stdin closes, saying it is `server_name` (default
/// `kwnt-echo`), with the Home Assistant-shaped tools too when `home_tools`.
pub async fn run_stdio(server_name: Option<&str>, home_tools: bool) -> std::io::Result<()> {
    let server_name = server_name.unwrap_or("kwnt-echo");
    let listed = {
        let mut all = tools();
        if home_tools {
            all.as_array_mut().unwrap().extend(home_tools_list());
        }
        all
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no answer
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(super::client::PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": server_name, "version": env!("CARGO_PKG_VERSION")},
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": listed}),
            "tools/call" => call(msg["params"]["name"].as_str().unwrap_or(""), &msg["params"]["arguments"]).await,
            _ => {
                let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}});
                stdout.write_all(format!("{err}\n").as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
        stdout.write_all(format!("{reply}\n").as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}
