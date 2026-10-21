//! The grok CLI driver: every run gets a private temp `GROK_HOME` and isolation env, keeps
//! only the user's model settings and login, and cleans up after itself.
#![cfg(unix)]

use kwnt::{
    db::models::Kwant,
    db::models::McpServer,
    drivers::{events::RuntimeEvent, CliDriver, Driver, TurnInput},
    mcp::{FixedApprover, GatewayAccess, McpTurnContext},
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

const USER_CONFIG: &str = r#"
[models]
default = "mock"

[model.mock]
model = "mock-model"
base_url = "http://127.0.0.1:1/v1"

[mcp_servers.grokuser]
command = "/bin/should-not-start"

[hooks]
SessionStart = "/bin/should-not-run"
"#;

const AUTH: &str = r#"{"xai::api_key":{"key":"xai-FAKE","auth_mode":"api_key"}}"#;

fn turn(mcp: Option<McpTurnContext>) -> TurnInput {
    TurnInput {
        continuity: String::new(),
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Ada".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "grok".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "hi".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
        mcp,
    }
}

/// A fake grok that records what it was given, then runs `body`.
struct FakeGrok {
    dir: TempDir,
    user_home: PathBuf,
}

impl FakeGrok {
    fn new(body: &str) -> Self {
        let dir = tempdir().unwrap();
        let user_home = dir.path().join("user-grok");
        std::fs::create_dir(&user_home).unwrap();
        std::fs::write(user_home.join("config.toml"), USER_CONFIG).unwrap();
        std::fs::write(user_home.join("auth.json"), AUTH).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let o = out.display();
        let script = format!(
            "#!/bin/sh\n\
             echo \"$GROK_HOME\" >> {o}/homes\n\
             env | grep -E '^(GROK_|KWNT_)' | sort > {o}/env\n\
             cp \"$GROK_HOME/config.toml\" {o}/config.toml\n\
             ls -la \"$GROK_HOME\" > {o}/ls\n\
             ls -ld \"$GROK_HOME\" >> {o}/ls\n\
             printf '%s\\n' \"$@\" > {o}/argv\n\
             {body}\n"
        );
        let path = dir.path().join("grok");
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir, user_home }
    }

    fn driver(&self) -> CliDriver {
        CliDriver::new(
            Some(self.dir.path().join("grok").to_string_lossy().into_owned()),
            Some("grok".into()),
        )
        .with_grok_home(self.user_home.clone())
    }

    fn out(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join("out").join(name)).unwrap_or_default()
    }

    fn homes(&self) -> Vec<PathBuf> {
        self.out("homes").lines().map(PathBuf::from).collect()
    }

    async fn run(&self, mcp: Option<McpTurnContext>) -> (String, Vec<String>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move {
            let mut notes = Vec::new();
            while let Some(ev) = rx.recv().await {
                if let RuntimeEvent::ThinkingDelta(t) = ev {
                    notes.push(t);
                }
            }
            notes
        });
        let out = self.driver().stream_turn(turn(mcp), tx).await.unwrap();
        (out.text, drain.await.unwrap())
    }
}

fn env_value(env: &str, key: &str) -> Option<String> {
    env.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

#[tokio::test]
async fn grok_runs_in_a_private_temp_home() {
    let grok = FakeGrok::new("echo hello");
    let (reply, _) = grok.run(None).await;
    assert_eq!(reply.trim(), "hello");

    let env = grok.out("env");
    let homes = grok.homes();
    assert_eq!(homes.len(), 1);
    let home = &homes[0];
    assert!(home.starts_with(std::env::temp_dir()), "{home:?}");
    assert_ne!(home, &grok.user_home);
    assert_eq!(
        env_value(&env, "GROK_HOME").map(PathBuf::from).as_ref(),
        Some(home)
    );
    for key in [
        "GROK_CLAUDE_MCPS_ENABLED",
        "GROK_CLAUDE_HOOKS_ENABLED",
        "GROK_CURSOR_MCPS_ENABLED",
        "GROK_CURSOR_HOOKS_ENABLED",
        "GROK_MANAGED_MCPS_ENABLED",
    ] {
        assert_eq!(env_value(&env, key).as_deref(), Some("0"), "{key} in {env}");
    }
    // The login is read in place; no copy of it is made
    assert_eq!(
        env_value(&env, "GROK_AUTH_PATH").map(PathBuf::from),
        Some(grok.user_home.join("auth.json"))
    );
    let listing = grok.out("ls");
    assert!(!listing.contains("auth.json"), "{listing}");
    assert!(
        listing.lines().last().unwrap().starts_with("drwx------"),
        "{listing}"
    );
    assert!(
        listing
            .lines()
            .any(|l| l.starts_with("-rw-------") && l.ends_with("config.toml")),
        "{listing}"
    );

    // Only the model settings are carried over, plus kwnt's shell env policy
    let config: toml::Table = grok.out("config.toml").parse().unwrap();
    assert_eq!(
        config.keys().collect::<Vec<_>>(),
        ["model", "models", "shell_environment_policy"]
    );
    assert_eq!(config["models"]["default"].as_str(), Some("mock"));
    assert_eq!(
        config["shell_environment_policy"]["inherit"].as_str(),
        Some("core")
    );

    assert!(!home.exists(), "temp home left behind: {home:?}");
    // The user's own grok home is untouched
    let mut user_files: Vec<_> = std::fs::read_dir(&grok.user_home)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    user_files.sort();
    assert_eq!(user_files, ["auth.json", "config.toml"]);
    assert_eq!(
        std::fs::read_to_string(grok.user_home.join("config.toml")).unwrap(),
        USER_CONFIG
    );
}

#[tokio::test]
async fn grok_falls_back_to_a_private_copy_of_the_login() {
    // A grok that ignores GROK_AUTH_PATH says it isn't signed in unless auth.json is in its home
    let grok = FakeGrok::new(
        "if [ -f \"$GROK_HOME/auth.json\" ] && [ -z \"$GROK_AUTH_PATH\" ]; then ls -l \"$GROK_HOME/auth.json\" | cut -c1-10; else echo 'Not signed in. To authenticate without a browser, run: grok login --device-code' >&2; exit 1; fi",
    );
    let (reply, _) = grok.run(None).await;
    assert_eq!(reply.trim(), "-rw-------", "copy is private");
    let homes = grok.homes();
    assert_eq!(homes.len(), 2, "one retry");
    assert_ne!(homes[0], homes[1]);
    assert!(homes.iter().all(|h| !h.exists()));
    assert_eq!(
        std::fs::read_to_string(grok.user_home.join("auth.json")).unwrap(),
        AUTH
    );
}

#[tokio::test]
async fn grok_without_a_login_file_runs_once_without_auth_env() {
    let grok = FakeGrok::new("echo 'Not signed in.' >&2; exit 1");
    std::fs::remove_file(grok.user_home.join("auth.json")).unwrap();
    grok.run(None).await;
    assert_eq!(grok.homes().len(), 1);
    assert_eq!(env_value(&grok.out("env"), "GROK_AUTH_PATH"), None);
}

#[tokio::test]
async fn grok_without_user_config_still_runs_isolated() {
    let grok = FakeGrok::new("echo ok");
    std::fs::remove_file(grok.user_home.join("config.toml")).unwrap();
    let (reply, _) = grok.run(None).await;
    assert_eq!(reply.trim(), "ok");
    // Nothing of the user's, but the model's shell is still limited to core env
    let config: toml::Table = grok.out("config.toml").parse().unwrap();
    assert_eq!(
        config.keys().collect::<Vec<_>>(),
        ["shell_environment_policy"]
    );
    assert_eq!(
        config["shell_environment_policy"]["inherit"].as_str(),
        Some("core")
    );
}

async fn wait_for(path: &Path) {
    for _ in 0..250 {
        if std::fs::read_to_string(path).is_ok_and(|s| s.ends_with('\n')) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{path:?} never appeared");
}

#[tokio::test]
async fn aborted_grok_turn_kills_grok_and_removes_its_home() {
    let grok = FakeGrok::new("echo $$ > \"$(dirname \"$0\")/out/pid\"; exec sleep 60");
    let driver = grok.driver();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let task = tokio::spawn(async move { driver.stream_turn(turn(None), tx).await });
    wait_for(&grok.dir.path().join("out/pid")).await;
    let home = grok.homes().remove(0);
    assert!(home.exists());
    task.abort();
    let _ = task.await;
    assert!(!home.exists(), "temp home left behind after abort");
    let pid = grok.out("pid").trim().to_string();
    for _ in 0..100 {
        // Gone, or a zombie waiting for tokio's orphan reaper
        let stat = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&stat.stdout);
        let alive = !stat.trim().is_empty() && !stat.trim().starts_with('Z');
        if !alive {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("grok (pid {pid}) outlived its aborted turn");
}

#[tokio::test]
async fn grok_killed_mid_run_still_removes_its_home() {
    let grok = FakeGrok::new("kill -TERM $$");
    grok.run(None).await;
    let homes = grok.homes();
    assert_eq!(homes.len(), 1);
    assert!(!homes[0].exists());
}

fn mcp_ctx(dir: &Path) -> McpTurnContext {
    let server = McpServer {
        id: "id-echo".into(),
        name: "echo".into(),
        transport: "stdio".into(),
        command: Some("/usr/bin/true".into()),
        args: vec![],
        url: None,
        env: BTreeMap::from([("API_TOKEN".to_string(), "sk-very-secret".to_string())]),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 30,
        created_at: None,
    };
    McpTurnContext {
        servers: vec![server],
        approver: Arc::new(FixedApprover(true)),
        config_dir: dir.join("mcp"),
        permission_url: None,
        gateway: Some(GatewayAccess {
            url: "http://127.0.0.1:1/api/mcp/gateway".into(),
            token: "gw-secret-token".into(),
        }),
        max_tool_iterations: 8,
        smart_approvals: false,
        records: Default::default(),
    }
}

#[tokio::test]
async fn grok_gets_the_gateway_with_the_token_in_env_only() {
    let grok = FakeGrok::new("echo ok");
    grok.run(Some(mcp_ctx(grok.dir.path()))).await;
    let argv = grok.out("argv");
    let config_text = grok.out("config.toml");
    let env = grok.out("env");
    assert!(
        argv.contains("--allow\nMCPTool(kwnt__*)\n--allow\nWebFetch\n--output-format\nstreaming-json\n"),
        "{argv}"
    );
    assert_eq!(
        env_value(&env, "KWNT_MCP_GATEWAY_TOKEN").as_deref(),
        Some("gw-secret-token")
    );
    let config: toml::Table = config_text.parse().unwrap();
    let servers = config["mcp_servers"].as_table().unwrap();
    assert_eq!(
        servers["kwnt"]["url"].as_str(),
        Some("http://127.0.0.1:1/api/mcp/gateway/grok"),
        "grok naming mode"
    );
    assert_eq!(
        servers.keys().collect::<Vec<_>>(),
        ["kwnt"],
        "only kwnt's gateway"
    );
    assert_eq!(
        servers["kwnt"]["headers"]["Authorization"].as_str(),
        Some("Bearer ${KWNT_MCP_GATEWAY_TOKEN}")
    );
    assert_eq!(
        config["shell_environment_policy"]["inherit"].as_str(),
        Some("core")
    );
    assert!(config["shell_environment_policy"]["exclude"]
        .as_array()
        .unwrap()
        .contains(&toml::Value::from("KWNT_MCP_GATEWAY_TOKEN")));
    for secret in ["gw-secret-token", "sk-very-secret", "/usr/bin/true"] {
        assert!(!argv.contains(secret), "{secret} in argv: {argv}");
        assert!(
            !config_text.contains(secret),
            "{secret} in config: {config_text}"
        );
    }
    assert!(!grok.homes()[0].exists());
}

#[tokio::test]
async fn grok_without_tools_gets_no_gateway() {
    let grok = FakeGrok::new("echo ok");
    grok.run(None).await;
    let argv = grok.out("argv");
    assert!(!argv.contains("MCPTool("), "{argv}");
    assert!(argv.contains("--allow\nWebFetch\n"), "grok may still read web pages: {argv}");
    assert!(argv.contains("--output-format\nstreaming-json\n"), "{argv}");
    assert_eq!(env_value(&grok.out("env"), "KWNT_MCP_GATEWAY_TOKEN"), None);
    assert!(!grok.out("config.toml").contains("mcp_servers"));
}

#[tokio::test]
async fn grok_reply_comes_from_streaming_json_text_events() {
    let grok = FakeGrok::new(
        r#"printf '%s\n' '{"type":"available_commands","tools":[]}' '{"type":"text","data":"Hel"}' '{"type":"tool_call","toolCallId":"c1"}' '{"type":"text","data":"lo ✓"}' '{"type":"end","stopReason":"end_turn"}'"#,
    );
    let (reply, _) = grok.run(None).await;
    assert_eq!(reply, "Hello ✓");
}

#[tokio::test]
async fn grok_refusing_a_tool_explains_instead_of_replying_empty() {
    let grok = FakeGrok::new(
        r#"echo 'permission required: run_terminal_cmd was not run, as a headless run cannot ask for approval' >&2; printf '%s\n' '{"type":"end","stopReason":"end_turn"}'"#,
    );
    let (reply, _) = grok.run(None).await;
    assert!(
        reply.contains("needed approval for “run_terminal_cmd”"),
        "{reply}"
    );
}

#[tokio::test]
async fn grok_crashing_without_output_reports_its_exit_and_hides_the_token() {
    let grok = FakeGrok::new(r#"echo "boom: Bearer $KWNT_MCP_GATEWAY_TOKEN" >&2; exit 3"#);
    let (reply, _) = grok.run(Some(mcp_ctx(grok.dir.path()))).await;
    assert!(
        reply.contains("exited with code 3") && reply.contains("boom"),
        "{reply}"
    );
    assert!(!reply.contains("gw-secret-token"), "{reply}");
}
