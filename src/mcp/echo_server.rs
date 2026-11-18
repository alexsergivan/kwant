//! A tiny stdio MCP server, run with the hidden `kwnt mcp-echo-server` subcommand. It exists so
//! tests and smoke checks have a real MCP server without Python, Node or network access.
//!
//! Tools: `echo {text}`, `add {a, b}`, `sleep {ms}`, `env {name}` (reads its own environment),
//! `crash` (exits mid-call) and `peek` (a fixed reading). `echo` is marked read-only; `add` is
//! marked destructive, so it asks; `peek` carries Home Assistant's read-only shape
//! (`readOnlyHint` and `destructiveHint` both true), which counts as read-only.
//!
//! `--home-tools` adds Home Assistant-shaped tools (26f): `homeassistant__GetLiveContext`
//! (read-only), `intent__HassTurnOn` and `intent__HassTurnOff` `{name, area, domain,
//! device_class}`, and `homeassistant__schedule_command {command}`; each change answers that it
//! did it. `GetLiveContext` answers like Home Assistant 2026.11.1 (27e): with no arguments it lists
//! the whole home ([`HOME`]); a `name`, `area` or `domain` filter that matches nothing is an error
//! (`isError: true`, `{"error": "No exposed entities found in domain(s): switch"}`), as on the
//! tester's install, where switches are lights.
//! `--server-name home-assistant` makes the server say it is Home Assistant's; it says
//! `kwnt-echo` otherwise.
//!
//! `--media-tools` adds FLUX-shaped tools (26h), all offline: `generate_image {count, prompt}`
//! answers with `count` (default 1, at most 12) small PNG pictures as `image` parts after a text
//! part; `get_credits` (read-only) answers with a balance; `get_result {url}` answers with a
//! delivery-style `{"status": "Ready", "result": {"sample": url}}`. Run it with
//! `--server-name flux` (and name the server `flux`) to look like Black Forest Labs' app.

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
         "annotations": {"destructiveHint": true}},
        {"name": "sleep", "description": "Waits for the given milliseconds, then answers.",
         "inputSchema": obj(json!({"ms": {"type": "integer"}}), json!(["ms"]))},
        {"name": "env", "description": "Returns the value of one of this server's environment variables.",
         "inputSchema": obj(json!({"name": {"type": "string"}}), json!(["name"]))},
        {"name": "crash", "description": "Exits the server without answering.",
         "inputSchema": obj(json!({}), json!([]))},
        {"name": "peek", "description": "Returns a fixed reading, like a sensor.",
         "inputSchema": obj(json!({}), json!([])),
         "annotations": {"readOnlyHint": true, "destructiveHint": true, "openWorldHint": false, "idempotentHint": false}},
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
         "inputSchema": obj(target.clone()), "annotations": {"readOnlyHint": true}},
        {"name": "intent__HassTurnOn", "description": "Turns on, opens or locks a device.", "inputSchema": obj(target.clone())},
        {"name": "intent__HassTurnOff", "description": "Turns off, closes or unlocks a device.", "inputSchema": obj(target)},
        {"name": "homeassistant__schedule_command", "description": "Runs a command later.",
         "inputSchema": obj(json!({"command": {"type": "string"}}))},
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
}

fn media_tools_list() -> Vec<Value> {
    let obj = |props: Value| json!({"type": "object", "properties": props});
    json!([
        {"name": "generate_image", "description": "Makes pictures from a prompt (offline stand-in).",
         "inputSchema": obj(json!({"prompt": {"type": "string"}, "count": {"type": "integer"}}))},
        {"name": "get_credits", "description": "Reads the credit balance.",
         "inputSchema": obj(json!({})), "annotations": {"readOnlyHint": true}},
        {"name": "get_result", "description": "Reads a finished generation.",
         "inputSchema": obj(json!({"url": {"type": "string"}})), "annotations": {"readOnlyHint": true}},
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
}

/// A 96×96 PNG: a diagonal gradient whose colours turn with `n`, so every picture differs.
pub fn sample_png(n: u8) -> Vec<u8> {
    const SIDE: u32 = 96;
    let mut raw = Vec::with_capacity(((SIDE * 3 + 1) * SIDE) as usize);
    for y in 0..SIDE {
        raw.push(0); // no filter
        for x in 0..SIDE {
            let t = ((x + y) * 255 / (2 * SIDE - 2)) as u8;
            let (r, g, b) = match n % 3 {
                0 => (116u8, 91u8.saturating_add(t / 2), 239u8.saturating_sub(t / 3)),
                1 => (t, 120, 255 - t),
                _ => (255 - t / 2, 180u8.saturating_sub(t / 2), t),
            };
            raw.extend_from_slice(&[r, g, b]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, &raw).ok();
    let idat = z.finish().unwrap_or_default();
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&SIDE.to_be_bytes());
    ihdr.extend_from_slice(&SIDE.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &idat);
    chunk(&mut png, b"IEND", &[]);
    png
}

/// The home `GetLiveContext` reads (27e): name, domain, area and state.
const HOME: &[(&str, &str, &str, &str)] = &[
    ("Living room lamp", "light", "Living room", "off"),
    ("Kitchen light", "light", "Kitchen", "on"),
    ("Porch light", "light", "Porch", "on"),
    ("Living room temperature", "sensor", "Living room", "21.4 °C"),
    ("Front door", "lock", "Hallway", "locked"),
];

/// A filter's values: a string or an array of strings; none when it is missing or empty.
fn filter_values(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_lowercase()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect(),
        _ => vec![],
    }
}

/// `GetLiveContext` as Home Assistant 2026.11.1 answers it (see the module docs).
fn live_context(args: &Value) -> Value {
    let (names, domains, areas) = (filter_values(&args["name"]), filter_values(&args["domain"]), filter_values(&args["area"]));
    let found: Vec<_> = HOME
        .iter()
        .filter(|(n, d, a, _)| {
            (names.is_empty() || names.contains(&n.to_lowercase()))
                && (domains.is_empty() || domains.contains(&d.to_string()))
                && (areas.is_empty() || areas.contains(&a.to_lowercase()))
        })
        .collect();
    if found.is_empty() {
        let error = if !names.is_empty() {
            format!("No exposed entities matched name '{}'", names.join("', '"))
        } else if !domains.is_empty() {
            format!("No exposed entities found in domain(s): {}", domains.join(", "))
        } else {
            format!("No exposed entities found in area(s): {}", areas.join(", "))
        };
        return text_result(json!({ "error": error }).to_string(), true);
    }
    let mut text = String::from("Live Context: An overview of the areas and the devices in this smart home:");
    for (name, domain, area, state) in found {
        text.push_str(&format!("\n- names: {name}\n  domain: {domain}\n  state: '{state}'\n  areas: {area}"));
    }
    text_result(json!({ "success": true, "result": text }).to_string(), false)
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

async fn call(name: &str, args: &Value) -> Value {
    match name {
        "homeassistant__GetLiveContext" => live_context(args),
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
        "peek" => text_result("21.4".into(), false),
        "generate_image" => {
            use base64::Engine;
            let count = args["count"].as_u64().unwrap_or(1).clamp(1, 12) as u8;
            let mut content = vec![json!({"type": "text", "text": format!("Generated {count} image(s) for: {}", args["prompt"].as_str().unwrap_or(""))})];
            for n in 0..count {
                let data = base64::engine::general_purpose::STANDARD.encode(sample_png(n));
                content.push(json!({"type": "image", "mimeType": "image/png", "data": data}));
            }
            json!({"content": content, "isError": false})
        }
        "get_credits" => text_result("{\"credits\": 412.5}".into(), false),
        "get_result" => text_result(json!({"status": "Ready", "result": {"sample": args["url"]}}).to_string(), false),
        "crash" => {
            eprintln!("mcp-echo-server: crashing on request");
            std::process::exit(3);
        }
        other => text_result(format!("unknown tool {other}"), true),
    }
}

/// Serves MCP on stdin/stdout until stdin closes, saying it is `server_name` (default
/// `kwnt-echo`), with the Home Assistant-shaped tools too when `home_tools` and the FLUX-shaped
/// ones when `media_tools`.
pub async fn run_stdio(server_name: Option<&str>, home_tools: bool, media_tools: bool) -> std::io::Result<()> {
    let server_name = server_name.unwrap_or("kwnt-echo");
    let listed = {
        let mut all = tools();
        if home_tools {
            all.as_array_mut().unwrap().extend(home_tools_list());
        }
        if media_tools {
            all.as_array_mut().unwrap().extend(media_tools_list());
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
