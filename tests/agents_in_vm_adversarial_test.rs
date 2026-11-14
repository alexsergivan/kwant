//! Test plan #1: the port of `proto/agents-in-vm/adversarial.sh` to the shipped code. A real
//! bot container (the `kwnt-agent` image, started by `AgentVm::prepare_turn`), the real broker
//! and real Claude Code, against a scripted mock model on 127.0.0.1. `probe.sh` tries the three
//! attacks (read a host sign-in, read the real token, reach the network, plus IPv6, UDP and DNS):
//!
//! * A. in a plain shell in the container;
//! * B. through the agent's own Bash tool, mid-turn, while the broker holds the real key;
//! * C. positive control: the real key reached the mock upstream, the stand-in never did;
//! * D. self-test: with a planted token the probe does report a leak;
//! * E. (#9) `docker inspect` of the running container matches the allow-list;
//! * F. (#14) a file the container writes belongs to this computer's user;
//! * G. (#12) symlinks the bot makes in the container never lead a host reader outside;
//! * H. (#16) `--resume` carries the conversation into the next turn through the broker;
//! * I. (#17) the real key is in no log, no `docker inspect` and no process list.
//!
//! Needs Docker and the agent image (`docker build -f docker/agent.Dockerfile --build-arg
//! CLAUDE_VERSION=… -t kwnt-agent:v1-claude-…`). Without them it says so and passes, unless
//! `KWNT_AGENT_TEST_REQUIRE=1` (QA runs) makes that a failure.
#![cfg(target_os = "linux")]

use axum::{body::Bytes, http::HeaderMap, response::IntoResponse, Router};
use kwnt::sandbox::agent_vm::{
    broker::{BrokerConfig, Credential, Secret},
    image_tag, AgentVm, TurnRequest, DUMMY_API_KEY,
};
use kwnt::config::AgentsInVmConfig;
use serde_json::{json, Value};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use tokio::process::Command;

const PROBE: &str = include_str!("../proto/agents-in-vm/probe.sh");

#[derive(Default, Clone)]
struct Upstream {
    /// The credential each model call arrived with.
    auth: Arc<Mutex<Vec<String>>>,
    /// Bash tool output the CLI sent back.
    tool_results: Arc<Mutex<Vec<String>>>,
    /// How many messages each model call carried, and its last user text.
    calls: Arc<Mutex<Vec<(usize, String)>>>,
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text_of).collect(),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("text") => o["text"].as_str().unwrap_or_default().to_string(),
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("tool_result") => text_of(o.get("content").unwrap_or(&Value::Null)),
        _ => String::new(),
    }
}

/// A scripted Messages API: a tool result is answered "done"; `RUN: <cmd>` with Bash offered is
/// answered with a Bash call; anything else with a line of text. Always SSE when asked.
async fn messages(up: Upstream, headers: HeaderMap, body: Bytes) -> axum::response::Response {
    let cred = headers
        .get("x-api-key")
        .or_else(|| headers.get("authorization"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    up.auth.lock().unwrap().push(cred);
    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    // The CLI may end the list with a `system` message of its own (the environment); the turn
    // is the last `user` message
    let last = req["messages"].as_array().and_then(|m| m.iter().rev().find(|m| m["role"] == "user")).cloned().unwrap_or(Value::Null);
    let blocks = match &last["content"] {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let results: Vec<String> = blocks.iter().filter(|b| b["type"] == "tool_result").map(text_of).collect();
    let prompt: String = blocks.iter().filter(|b| b["type"] != "tool_result").map(text_of).collect();
    let n = req["messages"].as_array().map_or(0, |m| m.iter().filter(|m| m["role"] != "system").count());
    up.calls.lock().unwrap().push((n, prompt.clone()));
    let offers_bash = req["tools"].as_array().is_some_and(|t| t.iter().any(|t| t["name"] == "Bash"));
    if std::env::var_os("KWNT_AGENT_TEST_DUMP").is_some() {
        let names: Vec<String> = req["tools"].as_array().map(|t| t.iter().map(|t| t["name"].as_str().unwrap_or("?").to_string()).collect()).unwrap_or_default();
        eprintln!("MOCK call: messages={n} bash={offers_bash} tools={names:?} last_role={} prompt={:?}", last["role"], prompt.chars().take(400).collect::<String>());
    }
    let tool = if !results.is_empty() {
        up.tool_results.lock().unwrap().extend(results);
        None
    } else if offers_bash && prompt.contains("RUN:") {
        let cmd = prompt.split_once("RUN:").unwrap().1.trim().lines().next().unwrap_or("").to_string();
        Some(json!({"command": cmd, "description": "adversarial probe", "timeout": 300000}))
    } else {
        None
    };
    let model = req["model"].as_str().unwrap_or("mock").to_string();
    let usage = json!({"input_tokens": 10, "output_tokens": 5});
    let mut ev = vec![json!({"type": "message_start", "message": {"id": "msg_mock", "type": "message", "role": "assistant",
        "model": model, "content": [], "stop_reason": null, "stop_sequence": null, "usage": usage}})];
    let stop = if let Some(input) = &tool {
        ev.push(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_mockprobe0001", "name": "Bash", "input": {}}}));
        ev.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}));
        "tool_use"
    } else {
        ev.push(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}));
        ev.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "done"}}));
        "end_turn"
    };
    ev.push(json!({"type": "content_block_stop", "index": 0}));
    ev.push(json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null}, "usage": {"output_tokens": 5}}));
    ev.push(json!({"type": "message_stop"}));
    if req["stream"] != true {
        let content = match tool {
            Some(input) => json!([{"type": "tool_use", "id": "toolu_mockprobe0001", "name": "Bash", "input": input}]),
            None => json!([{"type": "text", "text": "done"}]),
        };
        return axum::Json(json!({"id": "msg_mock", "type": "message", "role": "assistant", "model": model, "content": content,
            "stop_reason": stop, "stop_sequence": null, "usage": usage}))
        .into_response();
    }
    let sse: String = ev.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect();
    ([("content-type", "text/event-stream")], sse).into_response()
}

async fn start_upstream(up: Upstream) -> String {
    let app = Router::new().fallback(move |headers: HeaderMap, body: Bytes| messages(up.clone(), headers, body));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

fn docker_ok() -> bool {
    StdCommand::new("docker").args(["image", "inspect", &image_tag()]).output().is_ok_and(|o| o.status.success())
}

/// Removes the test's container whatever happens.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = StdCommand::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

async fn probe(container: &str, script: &Path, host_paths: &str, extra: &[&str], pre: &str) -> String {
    let mut c = Command::new("docker");
    c.args(["exec", "-e", &format!("HOST_PATHS={host_paths}")]);
    for e in extra {
        c.args(["-e", e]);
    }
    let out = c.arg(container).args(["sh", "-c", &format!("{pre}bash {}", script.display())]).output().await.unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_three_attacks_fail_in_a_shell_and_in_the_agents_own_bash() {
    if !docker_ok() {
        assert!(std::env::var("KWNT_AGENT_TEST_REQUIRE").is_err(), "Docker or the agent image {} is missing", image_tag());
        eprintln!("skipped: needs Docker and the agent image {}", image_tag());
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let up = Upstream::default();
    let mut cfg = BrokerConfig::new(data.path().join("broker.log"));
    cfg.anthropic_origin = start_upstream(up.clone()).await;
    cfg.oauth_token_url = format!("{}/v1/oauth/token", cfg.anthropic_origin);
    let vm = AgentVm::with_parts(data.path(), cfg, Some(image_tag()), AgentsInVmConfig::default(), None);

    // Every real credential in this test carries the probe's marker; the container must never see one
    let real = format!("sk-ant-api03-{}{}-{}", "REAL", "CANARY", uuid::Uuid::new_v4().simple());
    let turn = vm
        .prepare_turn(TurnRequest {
            bot_id: "adversarial".into(),
            folder: Some(work.path().to_path_buf()),
            credential: Credential::ApiKey(Secret::new(&real)),
            credential_home: None,
            kwnt_origin: None,
        })
        .await
        .unwrap_or_else(|r| panic!("the turn didn't start: {r:?}"));
    let _cleanup = Cleanup(turn.container().to_string());
    let script = turn.workdir().join("probe.sh");
    std::fs::write(&script, PROBE).unwrap();
    let home = dirs::home_dir().unwrap();
    let host_paths = [
        home.join(".claude/.credentials.json"),
        home.join(".codex/auth.json"),
        home.join(".grok/auth.json"),
        data.path().join("broker.log"),
        data.path().join("kwnt.db"),
    ]
    .iter()
    .map(|p| p.display().to_string())
    .collect::<Vec<_>>()
    .join(" ");

    // A. A plain shell in the container
    let a = probe(turn.container(), &script, &host_paths, &[], "").await;
    assert!(a.lines().any(|l| l == "PROBE_LEAKS=0"), "shell probe:\n{a}");

    // B. The agent's own Bash tool, mid-turn
    let mut claude = Command::new("claude");
    claude.args(["--model", "haiku", "--permission-mode", "default", "--allowedTools", "Bash", "--output-format", "stream-json", "--verbose", "-p"]);
    claude.arg(format!("RUN: HOST_PATHS='{host_paths}' bash {}", script.display()));
    let mut cmd = turn.wrap(&claude);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().unwrap();
    // I, part 1: the process list while the turn runs (after its first model call)
    for _ in 0..600 {
        if !up.auth.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let ps = StdCommand::new("ps").args(["-eww", "-o", "args"]).output().unwrap();
    let ps = String::from_utf8_lossy(&ps.stdout).into_owned();
    let out = tokio::time::timeout(std::time::Duration::from_secs(420), child.wait_with_output()).await.expect("claude finished").unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let results = up.tool_results.lock().unwrap().join("\n");
    assert!(results.lines().any(|l| l.trim() == "PROBE_LEAKS=0"), "agent probe:\n{results}\nclaude said:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(!stdout.contains(&real), "the real key never reaches the CLI's output");

    // C. Positive control: the real key reached the upstream, the stand-in never did
    let auth = up.auth.lock().unwrap().clone();
    assert!(auth.len() >= 2, "a tool turn is at least two model calls: {auth:?}");
    assert!(auth.iter().all(|a| a == &real), "only the real key reached the upstream ({} calls)", auth.len());
    assert!(!auth.iter().any(|a| a.contains(DUMMY_API_KEY)));
    let log = std::fs::read_to_string(data.path().join("broker.log")).unwrap();
    assert!(!log.contains(&real), "the broker log holds no secret");

    // D. Self-test: a planted token is caught, so the probe can fail
    let planted = format!("PLANTED={real}");
    let d = probe(turn.container(), &script, &host_paths, &[&planted], "echo \"$PLANTED\" > /tmp/planted; ").await;
    assert!(d.lines().any(|l| l.starts_with("PROBE_LEAKS=") && l != "PROBE_LEAKS=0"), "self-test missed the planted token:\n{d}");

    // E. The running container is configured as the allow-list says
    let inspect = StdCommand::new("docker").args(["inspect", turn.container()]).output().unwrap();
    let inspect_text = String::from_utf8_lossy(&inspect.stdout).into_owned();
    let info: Value = serde_json::from_str::<Value>(&inspect_text).unwrap()[0].clone();
    let hc = &info["HostConfig"];
    assert_eq!(hc["NetworkMode"], "none", "{hc}");
    assert_eq!(hc["CapDrop"], json!(["ALL"]), "{hc}");
    assert!(hc["CapAdd"].is_null() || hc["CapAdd"] == json!([]), "{hc}");
    assert_eq!(hc["ReadonlyRootfs"], true);
    assert_eq!(hc["Privileged"], false);
    assert!(hc["SecurityOpt"].as_array().unwrap().iter().any(|o| o.as_str().unwrap().starts_with("no-new-privileges")), "{hc}");
    assert_eq!(hc["Memory"], json!(AgentsInVmConfig::default().memory_mb * 1024 * 1024));
    let user = info["Config"]["User"].as_str().unwrap();
    assert!(!user.is_empty() && !user.starts_with("0:") && user != "0" && user != "root", "the container runs as {user}");
    let ids = format!("{}:{}", unsafe { libc::getuid() }, unsafe { libc::getgid() });
    assert_eq!(user, ids, "the container runs as this computer's user");
    let mounts = info["Mounts"].as_array().unwrap();
    assert_eq!(mounts.len(), 3, "exactly three mounts: {mounts:?}");
    let by_dst = |d: &str| mounts.iter().find(|m| m["Destination"] == d).unwrap_or_else(|| panic!("no {d} mount: {mounts:?}"));
    assert_eq!(by_dst(&turn.workdir().display().to_string())["RW"], true);
    assert_eq!(by_dst("/home/agent")["RW"], true);
    assert_eq!(by_dst("/run/kwnt")["RW"], false, "the broker's folder is read-only");
    for m in mounts {
        let src = m["Source"].as_str().unwrap();
        assert!(!src.contains("docker.sock") && src != "/var/run" && src != "/run", "{src}");
        assert_eq!(m["Type"], "bind");
    }

    // F. A file the container writes is this computer's user's, with an ordinary mode
    let made = turn.workdir().join("from-the-container.txt");
    let st = StdCommand::new("docker").args(["exec", turn.container(), "sh", "-c"]).arg(format!("echo hi > '{}'", made.display())).status().unwrap();
    assert!(st.success());
    let meta = std::fs::metadata(&made).unwrap();
    assert_eq!((meta.uid(), meta.gid()), (unsafe { libc::getuid() }, unsafe { libc::getgid() }));
    assert_eq!(meta.mode() & 0o600, 0o600, "the person can read and write it: {:o}", meta.mode());

    // G. Symlinks made in the container lead no host reader outside the folder
    let canary_dir = data.path().join("canary-dir");
    std::fs::create_dir_all(&canary_dir).unwrap();
    let canary = format!("CANARY-{}", uuid::Uuid::new_v4().simple());
    std::fs::write(canary_dir.join("secret.txt"), &canary).unwrap();
    std::fs::write(data.path().join("canary.txt"), &canary).unwrap();
    let links = format!(
        "cd '{w}' && ln -s '{f}' link-file && ln -s '{d}' link-dir && mkdir -p .git sub && ln -s '{f}' .git/HEAD && ln -s ../link-dir sub/up",
        w = turn.workdir().display(),
        f = data.path().join("canary.txt").display(),
        d = canary_dir.display()
    );
    assert!(StdCommand::new("docker").args(["exec", turn.container(), "sh", "-c", &links]).status().unwrap().success());
    let listing = kwnt::teams::discover::walk(turn.workdir(), std::time::Instant::now() + std::time::Duration::from_secs(10));
    let mut read = String::new();
    for e in &listing.entries {
        assert!(!e.path.starts_with("link-dir") && !e.path.starts_with("sub/up/"), "listed through a symlink: {}", e.path);
        if let Ok(b) = kwnt::teams::discover::read_file(turn.workdir(), &e.path, 1 << 20) {
            read.push_str(&String::from_utf8_lossy(&b));
        }
    }
    for rel in ["link-file", "link-dir/secret.txt", ".git/HEAD", "sub/up/secret.txt"] {
        if let Ok(b) = kwnt::teams::discover::read_file(turn.workdir(), rel, 1 << 20) {
            read.push_str(&String::from_utf8_lossy(&b));
        }
        let d = kwnt::sandbox::safe_fs::Dir::open(turn.workdir()).unwrap();
        assert!(d.open_file(Path::new(rel)).is_err(), "safe_fs opened {rel}");
    }
    assert!(!read.contains(&canary), "a host reader followed a symlink out of the folder");

    // H. --resume: the next turn carries the conversation, through the same broker
    let session = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| v["session_id"].as_str().map(String::from))
        .expect("the first turn names its session");
    let before = up.calls.lock().unwrap().len();
    let mut again = Command::new("claude");
    again.args(["--model", "haiku", "--output-format", "stream-json", "--verbose", "--resume", &session, "-p", "SECOND turn"]);
    let mut cmd = turn.wrap(&again);
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out2 = tokio::time::timeout(std::time::Duration::from_secs(300), cmd.output()).await.expect("resumed turn finished").unwrap();
    let calls = up.calls.lock().unwrap().clone();
    let resumed = calls[before..].iter().find(|(_, p)| p.contains("SECOND turn")).unwrap_or_else(|| {
        panic!("the resumed turn reached no model: {calls:?}\n{}", String::from_utf8_lossy(&out2.stderr))
    });
    assert!(resumed.0 >= 3, "the resumed turn carries the first one's messages: {calls:?}");

    // I, part 2: the key is in no log, `docker inspect`, process list or CLI home
    assert!(!ps.contains(&real), "the process list holds the key");
    assert!(!inspect_text.contains(&real), "docker inspect holds the key");
    let logs = StdCommand::new("docker").args(["logs", turn.container()]).output().unwrap();
    assert!(!String::from_utf8_lossy(&logs.stdout).contains(&real) && !String::from_utf8_lossy(&logs.stderr).contains(&real));
    let grep = StdCommand::new("grep").args(["-rlF", &real]).arg(data.path()).arg(turn.workdir()).output().unwrap();
    assert!(grep.stdout.is_empty(), "the key was written to {}", String::from_utf8_lossy(&grep.stdout));

    drop(turn);
    vm.remove_containers(|_| true).await;
}

fn running(container: &str) -> bool {
    StdCommand::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
}

/// Every process's command line in `container` (the image has no `ps`).
fn processes(container: &str) -> String {
    let list = r#"for p in /proc/[0-9]*; do tr '\0' ' ' < "$p/cmdline" 2>/dev/null; echo; done"#;
    let out = StdCommand::new("docker").args(["exec", container, "sh", "-c", list]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// M3 (R-9, S-9): what the bot starts with `setsid` leaves the turn's process group, so the
/// group's TERM at the turn's end misses it. While another turn uses the container it keeps
/// running; after the container's last turn the container is removed (after a 1 s grace), so
/// nothing the bot started runs on and its file stops growing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_the_bot_started_outlives_the_containers_last_turn() {
    if !docker_ok() {
        assert!(std::env::var("KWNT_AGENT_TEST_REQUIRE").is_err(), "Docker or the agent image {} is missing", image_tag());
        eprintln!("skipped: needs Docker and the agent image {}", image_tag());
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let vm = AgentVm::with_parts(data.path(), BrokerConfig::new(data.path().join("broker.log")), Some(image_tag()), AgentsInVmConfig::default(), None);
    let request = || TurnRequest {
        bot_id: "r9-escapees".into(),
        folder: Some(work.path().to_path_buf()),
        credential: Credential::ApiKey(Secret::new("sk-ant-api03-r9-test")),
        credential_home: None,
        kwnt_origin: None,
    };
    let first = vm.prepare_turn(request()).await.unwrap_or_else(|r| panic!("the turn didn't start: {r:?}"));
    let second = vm.prepare_turn(request()).await.unwrap_or_else(|r| panic!("the second turn didn't start: {r:?}"));
    let container = first.container().to_string();
    assert_eq!(second.container(), container, "both turns share the bot's container");
    let _cleanup = Cleanup(container.clone());
    let tick = first.workdir().join("tick");

    // The bot's escape: two processes out of every turn's process group
    let escape = format!(
        "setsid sh -c 'while :; do date >> {t}; sleep 0.2; done' </dev/null >/dev/null 2>&1 & \
         setsid sleep 3600 </dev/null >/dev/null 2>&1 &",
        t = tick.display()
    );
    assert!(StdCommand::new("docker").args(["exec", &container, "sh", "-c", &escape]).status().unwrap().success());
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let ps = processes(&container);
    assert!(ps.contains("sleep 3600"), "control: the escapees run\n{ps}");
    let grown = size(&tick);
    assert!(grown > 0, "control: tick grows");

    // Not the last turn: the container and what runs in it stay for the other turn
    drop(first);
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert!(running(&container), "the other turn still uses the container");
    assert!(size(&tick) > grown, "the escapee still runs while a turn uses the container");

    // The last turn: within the grace and a removal, nothing runs and tick stops
    drop(second);
    let ended = std::time::Instant::now();
    while running(&container) {
        assert!(ended.elapsed() < std::time::Duration::from_secs(15), "the container outlived its last turn");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    eprintln!("container stopped {:?} after the last turn ended", ended.elapsed());
    // `docker rm -f` kills first and deletes after, so the record goes a moment later
    let listed = || {
        let out = StdCommand::new("docker").args(["ps", "-aq", "--filter", &format!("name=^{container}$")]).output().unwrap();
        !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    };
    while listed() {
        assert!(ended.elapsed() < std::time::Duration::from_secs(15), "removed, not just stopped");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    eprintln!("container removed {:?} after the last turn ended", ended.elapsed());
    let at_end = size(&tick);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(size(&tick), at_end, "tick stopped growing");
    let host_ps = StdCommand::new("ps").args(["-eo", "args"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&host_ps.stdout).lines().any(|l| l.trim() == "sleep 3600"), "no escapee left on this computer");

    // The next turn gets a fresh container in the same folder
    let next = vm.prepare_turn(request()).await.unwrap_or_else(|r| panic!("the next turn didn't start: {r:?}"));
    assert!(running(next.container()));
    assert!(!processes(next.container()).contains("sleep 3600"), "nothing carried over");
    drop(next);
    vm.remove_containers(|_| true).await;
}

/// Answers each approval after `wait`, recording what it was asked.
struct Delayed {
    wait: std::time::Duration,
    answer: bool,
    asked: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl kwnt::mcp::ToolApprover for Delayed {
    async fn approve(&self, r: kwnt::mcp::ToolApprovalRequest) -> bool {
        self.asked.lock().unwrap().push(r.tool_name.clone());
        tokio::time::sleep(self.wait).await;
        self.answer
    }
}

/// One turn with kwnt's approval tool (`--permission-prompt-tool`), as the Claude driver runs
/// it: the Bash call goes to kwnt's permission route and back. What was asked, whether the
/// command ran, and how many model calls the broker passed.
async fn approval_turn(name: &str, answer: bool) -> (Vec<String>, bool, usize, String) {
    use kwnt::{broker::PermissionBroker, config::Config, db::Database, router::LayaRouter, server::mcp_runtime::PermissionSession, server::AppState, tools::ToolRunner};
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = kwnt::server::create_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let up = Upstream::default();
    let mut cfg = BrokerConfig::new(data.path().join("broker.log"));
    cfg.anthropic_origin = start_upstream(up.clone()).await;
    cfg.oauth_token_url = format!("{}/v1/oauth/token", cfg.anthropic_origin);
    let vm = AgentVm::with_parts(data.path(), cfg, Some(image_tag()), AgentsInVmConfig::default(), None);
    let mut turn = vm
        .prepare_turn(TurnRequest {
            bot_id: format!("adversarial-{name}"),
            folder: Some(work.path().to_path_buf()),
            credential: Credential::ApiKey(Secret::new("sk-ant-api03-APPROVALTEST")),
            credential_home: None,
            kwnt_origin: Some(base.clone()),
        })
        .await
        .unwrap_or_else(|r| panic!("the turn didn't start: {r:?}"));
    let _cleanup = Cleanup(turn.container().to_string());

    let asked = Arc::new(Mutex::new(Vec::new()));
    let approver = Arc::new(Delayed { wait: std::time::Duration::from_millis(200), answer, asked: asked.clone() });
    let token = format!("tok-{name}-{}", uuid::Uuid::new_v4().simple());
    let ctx = kwnt::mcp::McpTurnContext {
        servers: vec![],
        approver,
        config_dir: tmp.path().join("mcp"),
        permission_url: Some(format!("{base}/api/mcp/permission/{token}")),
        gateway: None,
        max_tool_iterations: 8,
        approvals: kwnt::mcp::Approvals::Ask,
        records: Default::default(),
        kwnt_only: false,
    };
    state
        .mcp_sessions
        .lock()
        .await
        .insert(token, PermissionSession::new("k1".into(), "Adversarial".into(), "t1".into(), ctx.clone(), Some(turn.workdir().to_path_buf())));
    let (cfg_path, _dropped) = turn.write_mcp_config(&ctx).unwrap();

    let marker = format!("approval-marker-{name}");
    let mut claude = Command::new("claude");
    claude.args(["--model", "haiku", "--permission-mode", "default", "--output-format", "stream-json", "--verbose"]);
    claude.args(["--strict-mcp-config", "--mcp-config", &cfg_path, "--permission-prompt-tool", "mcp__kwnt__approve", "-p"]);
    claude.arg(format!("RUN: touch {marker}"));
    claude.env("MCP_TOOL_TIMEOUT", "330000").env("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "330000").env("MCP_TIMEOUT", "30000");
    let mut cmd = turn.wrap(&claude);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(420), cmd.spawn().unwrap().wait_with_output()).await.expect("claude finished").unwrap();
    let ran = turn.workdir().join(&marker).exists() || work.path().join(&marker).exists();
    let said = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let calls = up.calls.lock().unwrap().len();
    drop(turn);
    vm.remove_containers(|_| true).await;
    let asked = asked.lock().unwrap().clone();
    (asked, ran, calls, said)
}

/// QA F-1: with kwnt's approval tool on (every real Claude turn), the request carries the
/// AskUserQuestion schema, whose `metadata.properties.source` the broker once took for a fetch
/// by reference and refused. Every model call now passes, and Allow and Deny both work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approval_goes_through_kwnt_and_both_answers_work() {
    if !docker_ok() {
        assert!(std::env::var("KWNT_AGENT_TEST_REQUIRE").is_err(), "Docker or the agent image {} is missing", image_tag());
        eprintln!("skipped: needs Docker and the agent image {}", image_tag());
        return;
    }
    let (asked, ran, calls, said) = approval_turn("allow", true).await;
    assert_eq!(asked, vec!["Bash".to_string()], "the Bash call asked kwnt once:\n{said}");
    assert!(ran, "allowed: the command ran:\n{said}");
    assert!(calls >= 2, "the broker passed the call that asked and the one after the answer:\n{said}");
    let (asked, ran, calls, said) = approval_turn("deny", false).await;
    assert_eq!(asked, vec!["Bash".to_string()], "{said}");
    assert!(!ran, "denied: the command didn't run:\n{said}");
    assert!(calls >= 2, "{said}");
}
