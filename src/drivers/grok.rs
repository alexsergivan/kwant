//! Isolation for grok CLI runs. grok loads MCP servers, plugins and hooks from its own home
//! (`$GROK_HOME`, default `~/.grok`) and imports Claude's and Cursor's servers and hooks, so
//! headless runs would start the user's own servers with no approval card. Every run gets a
//! fresh private `GROK_HOME` holding only the user's model settings; the login is read in place
//! through `GROK_AUTH_PATH`. The directory is removed when the run ends, including on cancel.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Env switches that stop grok importing Claude/Cursor servers and hooks, and managed servers.
pub const ISOLATION_ENV: [(&str, &str); 6] = [
    ("GROK_CLAUDE_MCPS_ENABLED", "0"),
    ("GROK_CLAUDE_HOOKS_ENABLED", "0"),
    ("GROK_CURSOR_MCPS_ENABLED", "0"),
    ("GROK_CURSOR_HOOKS_ENABLED", "0"),
    ("GROK_MANAGED_MCPS_ENABLED", "0"),
    ("GROK_DISABLE_AUTOUPDATER", "1"),
];

/// Inherited env grok would read config from instead of the temp home.
pub const REMOVED_ENV: [&str; 1] = ["GROK_CONFIG"];

/// Tables copied from the user's `config.toml`: the default model and model definitions.
const KEPT_TABLES: [&str; 2] = ["models", "model"];

const DIR_PREFIX: &str = "kwnt-grok-";

/// Left behind only if kwnt itself was killed mid-run; no turn lasts this long.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The user's real grok home: `$GROK_HOME`, else `~/.grok`.
pub fn user_grok_home() -> Option<PathBuf> {
    match std::env::var_os("GROK_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::home_dir().map(|h| h.join(".grok")),
    }
}

/// The parts of the user's `config.toml` a kwnt run may use. Servers, plugins, hooks,
/// permissions and everything else are dropped. Unreadable config gives an empty table.
pub fn model_settings(user_config: &str) -> toml::Table {
    let parsed: toml::Table = match user_config.parse() {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!(
                "could not parse the user's grok config.toml; using grok's default model settings"
            );
            return toml::Table::new();
        }
    };
    parsed
        .into_iter()
        .filter(|(k, _)| KEPT_TABLES.contains(&k.as_str()))
        .collect()
}

/// A private, per-run `GROK_HOME`. Dropping it removes the directory.
#[derive(Debug)]
pub struct GrokHome {
    pub path: PathBuf,
}

impl Drop for GrokHome {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, dir = %self.path.display(), "could not remove grok temp home");
            }
        }
    }
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

impl GrokHome {
    /// Creates `<base>/kwnt-grok-<uuid>` (0700) with `config` as its `config.toml` (0600).
    pub fn create(base: &Path, config: &toml::Table) -> std::io::Result<Self> {
        sweep_stale(base);
        let path = base.join(format!("{DIR_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        let home = Self { path };
        let text = toml::to_string(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_private(&home.path.join("config.toml"), text.as_bytes())?;
        Ok(home)
    }

    /// Fallback login: a 0600 copy of the user's `auth.json`, removed with the home.
    pub fn copy_auth(&self, auth: &Path) -> std::io::Result<()> {
        write_private(&self.path.join("auth.json"), &std::fs::read(auth)?)
    }
}

/// Removes temp homes left by a kwnt that was killed before it could clean up.
fn sweep_stale(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(DIR_PREFIX))
            && entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_AFTER);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// grok's message when no login was found; the run did nothing, so it can be retried.
pub fn not_signed_in(stderr: &str) -> bool {
    stderr.contains("Not signed in")
}

/// Longest piece of grok's stderr quoted in a notice.
const NOTICE_TAIL_CHARS: usize = 600;

/// What a grok run that produced no reply left behind, for [`empty_reply_notice`].
#[derive(Debug, Default)]
pub struct EmptyRun<'a> {
    pub stderr: &'a str,
    /// grok's `error` events.
    pub errors: &'a [String],
    pub stop_reason: Option<&'a str>,
    /// `None` when grok was killed by a signal or its status is unknown.
    pub exit_code: Option<i32>,
}

/// Tools grok refused in headless mode: `permission required: <tool> was not run, …`.
fn refused_tools(stderr: &str) -> Vec<String> {
    let mut tools: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let Some(rest) = line
            .split_once("permission required:")
            .map(|(_, r)| r.trim())
        else {
            continue;
        };
        let tool = rest
            .split_once(" was not run")
            .map_or(rest, |(t, _)| t)
            .trim()
            .to_string();
        if !tool.is_empty() && !tools.contains(&tool) {
            tools.push(tool);
        }
    }
    tools
}

/// The message shown instead of an empty reply: why grok gave none, with any quoted output
/// redacted (`secrets`: the gateway token and the kwant's server secrets) and shortened.
pub fn empty_reply_notice(run: &EmptyRun<'_>, secrets: &[String]) -> String {
    let quote = |text: &str| {
        let text = crate::mcp::redact(text.trim(), secrets);
        let start = text
            .char_indices()
            .rev()
            .nth(NOTICE_TAIL_CHARS - 1)
            .map_or(0, |(i, _)| i);
        if start > 0 {
            format!("…{}", &text[start..])
        } else {
            text
        }
    };
    let refused = refused_tools(run.stderr);
    if !refused.is_empty() {
        return format!(
            "grok stopped without a reply: it needed approval to run {}, and a headless grok run can't ask for it.              kwnt only approves MCP tools assigned to this kwant (through its approval card), not grok's own tools.",
            refused.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ")
        );
    }
    if not_signed_in(run.stderr) {
        return "grok isn't signed in on this computer. Run `grok login` (or `grok login --device-code` without a browser), then try again.".into();
    }
    if let Some(error) = run.errors.first() {
        return format!("grok stopped with an error: {}", quote(error));
    }
    if run.stop_reason == Some("refusal") {
        return "grok declined to answer this request.".into();
    }
    let ended = match (run.exit_code, run.stop_reason) {
        (Some(0), Some(reason)) => {
            format!("grok finished without a reply (stop reason: {reason}).")
        }
        (Some(0), None) => "grok finished without a reply.".to_string(),
        (Some(code), _) => format!("grok exited with code {code} without a reply."),
        (None, _) => "grok was stopped before it replied.".to_string(),
    };
    match run.stderr.trim() {
        "" => ended,
        tail => format!("{ended} Its last output:\n{}", quote(tail)),
    }
}

/// Adds kwnt's gateway as the run's only MCP server. The token is not written: grok expands
/// `${VAR}` in headers from its own env, and the shell policy hides that variable from the
/// commands grok runs. `slowest` is the longest timeout among the kwant's servers.
pub fn add_gateway(config: &mut toml::Table, url: &str, slowest: u64) {
    let token_env = crate::mcp::GATEWAY_TOKEN_ENV;
    // The gateway connects to every server on grok's first tools/list
    let startup_secs = (slowest + 15).max(30);
    // A call may wait for the user's approval; grok must not give up first
    let tool_secs = (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest);
    let mut headers = toml::Table::new();
    headers.insert(
        "Authorization".into(),
        format!("Bearer ${{{token_env}}}").into(),
    );
    let mut server = toml::Table::new();
    server.insert("url".into(), url.into());
    server.insert("headers".into(), headers.into());
    server.insert("startup_timeout_sec".into(), (startup_secs as i64).into());
    server.insert("tool_timeout_sec".into(), (tool_secs as i64).into());
    let mut servers = toml::Table::new();
    servers.insert(crate::mcp::RESERVED_SERVER_NAME.into(), server.into());
    config.insert("mcp_servers".into(), servers.into());
    let mut policy = toml::Table::new();
    policy.insert("exclude".into(), toml::Value::Array(vec![token_env.into()]));
    config.insert("shell_environment_policy".into(), policy.into());
}

/// The `--allow` rule that lets grok call the gateway's tools without its own (headless,
/// always-refused) approval prompt. kwnt's approval card is shown by the gateway instead.
pub fn gateway_allow_rule() -> String {
    format!("MCPTool({}__*)", crate::mcp::RESERVED_SERVER_NAME)
}

/// One event of grok's `--output-format streaming-json` that kwnt acts on.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A chunk of the reply.
    Text(String),
    /// grok reported an error.
    Error(String),
    /// The run ended; `stop_reason` is e.g. `end_turn`, `refusal`, `cancelled`.
    End { stop_reason: Option<String> },
}

/// Splits grok's NDJSON output into events. Lines that aren't JSON are passed on as text, so
/// a grok that ignores `--output-format` still gets its reply through.
#[derive(Debug, Default)]
pub struct StreamParser {
    pending: String,
}

impl StreamParser {
    pub fn push(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.pending.push_str(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=end).collect();
            events.extend(parse_line(&line));
        }
        events
    }

    /// Whatever is left after the last newline.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let rest = std::mem::take(&mut self.pending);
        parse_line(&rest).into_iter().collect()
    }
}

fn parse_line(line: &str) -> Option<StreamEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Some(StreamEvent::Text(line.to_string()));
    };
    match value["type"].as_str() {
        Some("text") => value["data"]
            .as_str()
            .map(|t| StreamEvent::Text(t.to_string())),
        Some("error") => Some(StreamEvent::Error(
            value["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string(),
        )),
        Some("end") => Some(StreamEvent::End {
            stop_reason: value["stopReason"].as_str().map(str::to_string),
        }),
        Some(_) => None,
        None => Some(StreamEvent::Text(line.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CONFIG: &str = r#"
[models]
default = "mock"

[model.mock]
model = "mock-model"
base_url = "http://127.0.0.1:1/v1"
api_key = "user-key"

[mcp_servers.grokuser]
command = "/bin/evil"

[hooks]
x = 1

[plugins.p]
enabled = true

[permission]
allow = ["Bash(*)"]
"#;

    #[test]
    fn gateway_config_keeps_the_token_out_of_the_file() {
        let mut config = model_settings(USER_CONFIG);
        add_gateway(&mut config, "http://127.0.0.1:7/api/mcp/gateway/grok", 400);
        let text = toml::to_string(&config).unwrap();
        let parsed: toml::Table = text.parse().unwrap();
        let kwnt = &parsed["mcp_servers"]["kwnt"];
        assert_eq!(
            parsed["mcp_servers"].as_table().unwrap().len(),
            1,
            "only the gateway: {text}"
        );
        assert_eq!(
            kwnt["url"].as_str(),
            Some("http://127.0.0.1:7/api/mcp/gateway/grok")
        );
        assert_eq!(
            kwnt["headers"]["Authorization"].as_str(),
            Some("Bearer ${KWNT_MCP_GATEWAY_TOKEN}")
        );
        assert_eq!(kwnt["startup_timeout_sec"].as_integer(), Some(415));
        assert_eq!(kwnt["tool_timeout_sec"].as_integer(), Some(400));
        assert_eq!(
            parsed["shell_environment_policy"]["exclude"]
                .as_array()
                .unwrap(),
            &vec![toml::Value::from("KWNT_MCP_GATEWAY_TOKEN")]
        );
        assert_eq!(parsed["models"]["default"].as_str(), Some("mock"));

        // Short server timeouts still leave room for the approval wait
        let mut config = toml::Table::new();
        add_gateway(&mut config, "http://x", 0);
        assert_eq!(
            config["mcp_servers"]["kwnt"]["startup_timeout_sec"].as_integer(),
            Some(30)
        );
        assert_eq!(
            config["mcp_servers"]["kwnt"]["tool_timeout_sec"].as_integer(),
            Some(330)
        );
        assert_eq!(gateway_allow_rule(), "MCPTool(kwnt__*)");
    }

    #[test]
    fn written_gateway_home_never_contains_the_token() {
        let base = tempfile::tempdir().unwrap();
        let mut config = model_settings(USER_CONFIG);
        add_gateway(&mut config, "http://127.0.0.1:7/api/mcp/gateway/grok", 30);
        let home = GrokHome::create(base.path(), &config).unwrap();
        let written = std::fs::read_to_string(home.path.join("config.toml")).unwrap();
        assert!(written.contains("${KWNT_MCP_GATEWAY_TOKEN}"), "{written}");
        assert!(!written.contains("grokuser"));
        for entry in std::fs::read_dir(&home.path).unwrap() {
            assert_eq!(entry.unwrap().file_name(), "config.toml");
        }
    }

    #[test]
    fn parses_streaming_json_across_chunks() {
        let mut parser = StreamParser::default();
        let mut events =
            parser.push("{\"type\":\"available_commands\",\"tools\":[]}\n{\"type\":\"text\",\"da");
        events.extend(parser.push("ta\":\"Hel\"}\n{\"type\":\"text\",\"data\":\"lo\\n\"}\n"));
        events.extend(parser.push("{\"type\":\"tool_call\",\"toolCallId\":\"c1\"}\nplain line\n"));
        events.extend(parser.push("{\"type\":\"error\",\"message\":\"boom\"}\n{\"type\":\"end\",\"stopReason\":\"end_turn\"}"));
        events.extend(parser.finish());
        assert_eq!(
            events,
            [
                StreamEvent::Text("Hel".into()),
                StreamEvent::Text("lo\n".into()),
                StreamEvent::Text("plain line\n".into()),
                StreamEvent::Error("boom".into()),
                StreamEvent::End {
                    stop_reason: Some("end_turn".into())
                },
            ]
        );
        assert!(parser.finish().is_empty());
    }

    #[test]
    fn empty_reply_explains_refused_tools() {
        let stderr = "warn: x\npermission required: run_terminal_cmd was not run, as a headless run cannot ask for approval\npermission required: kwnt__alpha-ping was not run, as a headless run cannot ask for approval\npermission required: run_terminal_cmd was not run, as a headless run cannot ask for approval\n";
        let notice = empty_reply_notice(
            &EmptyRun {
                stderr,
                exit_code: Some(0),
                ..Default::default()
            },
            &[],
        );
        assert!(
            notice.contains("needed approval to run `run_terminal_cmd`, `kwnt__alpha-ping`,"),
            "{notice}"
        );
    }

    #[test]
    fn empty_reply_explains_why_and_hides_secrets() {
        let secrets = ["gw-token-123".to_string(), "sk-server".to_string()];
        let notice = |run: EmptyRun<'_>| empty_reply_notice(&run, &secrets);

        let login = notice(EmptyRun {
            stderr:
                "Not signed in. To authenticate without a browser, run: grok login --device-code",
            exit_code: Some(1),
            ..Default::default()
        });
        assert!(
            login.contains("isn't signed in") && login.contains("grok login"),
            "{login}"
        );

        let errors = ["upstream said Bearer gw-token-123 is bad".to_string()];
        let error = notice(EmptyRun {
            errors: &errors,
            exit_code: Some(1),
            ..Default::default()
        });
        assert_eq!(
            error,
            "grok stopped with an error: upstream said Bearer ******** is bad"
        );

        let refusal = notice(EmptyRun {
            stop_reason: Some("refusal"),
            exit_code: Some(0),
            ..Default::default()
        });
        assert_eq!(refusal, "grok declined to answer this request.");

        assert_eq!(
            notice(EmptyRun {
                exit_code: Some(0),
                ..Default::default()
            }),
            "grok finished without a reply."
        );
        assert_eq!(
            notice(EmptyRun {
                stop_reason: Some("max_turn_requests"),
                exit_code: Some(0),
                ..Default::default()
            }),
            "grok finished without a reply (stop reason: max_turn_requests)."
        );
        assert_eq!(
            notice(EmptyRun {
                exit_code: None,
                ..Default::default()
            }),
            "grok was stopped before it replied."
        );

        let crash = notice(EmptyRun {
            stderr: "panic: key sk-server leaked\n",
            exit_code: Some(101),
            ..Default::default()
        });
        assert_eq!(crash, "grok exited with code 101 without a reply. Its last output:\npanic: key ******** leaked");

        let long = format!("{}END", "x".repeat(5000));
        let tail = notice(EmptyRun {
            stderr: &long,
            exit_code: Some(2),
            ..Default::default()
        });
        assert!(
            tail.ends_with("xxEND") && tail.contains('…') && tail.chars().count() < 700,
            "{}",
            tail.len()
        );
    }

    #[test]
    fn keeps_only_model_settings() {
        let kept = model_settings(USER_CONFIG);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["model", "models"]);
        assert_eq!(kept["models"]["default"].as_str(), Some("mock"));
        assert_eq!(
            kept["model"]["mock"]["base_url"].as_str(),
            Some("http://127.0.0.1:1/v1")
        );
        assert!(model_settings("not [toml").is_empty());
        assert!(model_settings("").is_empty());
    }

    #[test]
    fn temp_home_is_private_and_removed_on_drop() {
        let base = tempfile::tempdir().unwrap();
        let auth = base.path().join("real-auth.json");
        std::fs::write(&auth, "{}").unwrap();
        let home = GrokHome::create(base.path(), &model_settings(USER_CONFIG)).unwrap();
        let path = home.path.clone();
        assert!(path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(DIR_PREFIX));
        let written = std::fs::read_to_string(path.join("config.toml")).unwrap();
        assert!(
            written.contains("mock-model")
                && !written.contains("grokuser")
                && !written.contains("hooks"),
            "{written}"
        );
        home.copy_auth(&auth).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o700);
            assert_eq!(mode(&path.join("config.toml")), 0o600);
            assert_eq!(mode(&path.join("auth.json")), 0o600);
        }
        drop(home);
        assert!(!path.exists());
        assert!(auth.exists(), "the user's own file is untouched");
    }

    #[test]
    fn sweeps_only_stale_kwnt_homes() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join(format!("{DIR_PREFIX}old"));
        let other = base.path().join("someone-else");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&other).unwrap();
        let past = std::time::SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        let fresh = GrokHome::create(base.path(), &toml::Table::new()).unwrap();
        assert!(!old.exists() && other.exists() && fresh.path.exists());
    }
}
