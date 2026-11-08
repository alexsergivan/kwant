//! Gate 0, part 2: one isolated Claude turn and a `--resume` turn against the REAL Anthropic API
//! through kwnt's own (Rust) broker, so the allow-list, the forwarded headers and the betas taken
//! from `proto/agents-in-vm/evidence/real-upstream.txt` are shown to be enough.
//!
//! Runs only with `KWNT_GATE0_KEY_FILE=<path to an Anthropic API key>` (it spends about four
//! Haiku calls), plus Docker and the agent image. The key is read here, held by the broker only,
//! and never printed.
#![cfg(target_os = "linux")]

use kwnt::config::AgentsInVmConfig;
use kwnt::sandbox::agent_vm::{
    broker::{BrokerConfig, Credential, Secret},
    image_tag, AgentVm, TurnRequest,
};
use serde_json::Value;
use std::process::Command as StdCommand;
use tokio::process::Command;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_turn_and_a_resume_pass_through_kwnts_broker() {
    let Some(key_file) = std::env::var_os("KWNT_GATE0_KEY_FILE") else {
        eprintln!("skipped: set KWNT_GATE0_KEY_FILE to run against the real API");
        return;
    };
    let key = std::fs::read_to_string(&key_file).expect("the key file").trim().to_string();
    assert!(StdCommand::new("docker").args(["image", "inspect", &image_tag()]).output().is_ok_and(|o| o.status.success()));
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let vm = AgentVm::with_parts(data.path(), BrokerConfig::new(data.path().join("broker.log")), Some(image_tag()), AgentsInVmConfig::default(), None);
    let turn = vm
        .prepare_turn(TurnRequest {
            bot_id: "gate0-rust".into(),
            folder: Some(work.path().to_path_buf()),
            credential: Credential::ApiKey(Secret::new(&key)),
            credential_home: None,
            kwnt_origin: None,
        })
        .await
        .unwrap_or_else(|r| panic!("the turn didn't start: {r:?}"));

    // The flags kwnt's CLI driver gives an isolated turn that matter here
    let base = ["--model", "haiku", "--strict-mcp-config", "--disallowedTools", "WebSearch,WebFetch", "--output-format", "stream-json", "--verbose"];
    let mut first = Command::new("claude");
    first.args(base).args(["--allowedTools", "Bash", "-p", "Use the Bash tool to run: echo rust-broker-ok > rb.txt. Then reply with only the file's content."]);
    let out = tokio::time::timeout(std::time::Duration::from_secs(300), turn.wrap(&first).stdin(std::process::Stdio::null()).output()).await.unwrap().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let result = |s: &str| s.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["type"] == "result").unwrap_or(Value::Null);
    let r1 = result(&stdout);
    assert_eq!(r1["is_error"], false, "first turn: {}", r1["subtype"]);
    assert_eq!(std::fs::read_to_string(work.path().join("rb.txt")).unwrap().trim(), "rust-broker-ok");
    let session = r1["session_id"].as_str().expect("a session id").to_string();

    let mut again = Command::new("claude");
    again.args(base).args(["--resume", &session, "-p", "What did the file contain? Reply with only that."]);
    let out2 = tokio::time::timeout(std::time::Duration::from_secs(300), turn.wrap(&again).stdin(std::process::Stdio::null()).output()).await.unwrap().unwrap();
    let stdout2 = String::from_utf8_lossy(&out2.stdout).into_owned();
    let r2 = result(&stdout2);
    assert_eq!(r2["is_error"], false, "resumed turn: {}", r2["subtype"]);
    assert!(r2["result"].as_str().unwrap_or_default().contains("rust-broker-ok"), "the resumed turn remembers the first");

    // Every call was a listed one, answered 200, with no beta stripped; nothing holds the key
    let log = std::fs::read_to_string(data.path().join("broker.log")).unwrap();
    let recs: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(recs.len() >= 3, "{} calls", recs.len());
    for r in &recs {
        assert_eq!((r["route"].as_str(), r["path"].as_str(), r["status"].as_u64()), (Some("anthropic"), Some("/v1/messages?beta=true"), Some(200)), "{r}");
        assert!(r.get("beta_stripped").is_none_or(|b| b.as_array().is_some_and(Vec::is_empty)), "a beta the CLI needs was stripped: {r}");
    }
    eprintln!("gate0-rust: {} calls through kwnt's broker, all POST /v1/messages?beta=true 200, no beta stripped", recs.len());
    for (what, text) in [("broker log", &log), ("first turn output", &stdout), ("resumed turn output", &stdout2)] {
        assert!(!text.contains(&key), "the key is in the {what}");
    }
    let grep = StdCommand::new("grep").args(["-rlF", "-f", "/dev/stdin"]).arg(data.path()).arg(work.path()).stdin(std::process::Stdio::piped()).spawn().and_then(|mut c| {
        use std::io::Write;
        c.stdin.take().unwrap().write_all(key.as_bytes())?;
        c.wait_with_output()
    });
    assert!(grep.unwrap().stdout.is_empty(), "the key was written to a file");
    drop(turn);
    vm.remove_containers(|_| true).await;
}
