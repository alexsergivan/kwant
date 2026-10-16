pub mod api;
pub mod cli;
pub mod events;
pub mod grok;

pub use api::ApiDriver;
pub use cli::{find_cli_binary, CliDriver};
pub use events::{RoutingCandidate, RuntimeEvent};

use crate::db::models::{Kwant, Message};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Error, Debug)]
pub enum DriverError {
    #[error("I/O or Subprocess error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP API error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Driver execution error: {0}")]
    Execution(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamContext {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workspace_path: Option<String>,
    pub repo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInput {
    pub thread_id: String,
    pub kwant: Kwant,
    pub prompt: String,
    pub history: Vec<Message>,
    pub model_override: Option<String>,
    pub effort_level: Option<String>,
    pub team_context: Option<TeamContext>,
    pub workspace_dir: Option<std::path::PathBuf>,
    #[serde(default)]
    pub browser_api_url: Option<String>,
    /// Sandbox whose browser is shown to the user for this conversation (team, channel or kwant).
    #[serde(default)]
    pub sandbox_id: Option<String>,
    /// Other kwants in this chat that the speaker can hand the turn to with an @mention.
    #[serde(default)]
    pub teammates: Vec<Teammate>,
    /// MCP servers, approver and records for this turn; `None` when the kwant has none.
    #[serde(skip)]
    pub mcp: Option<crate::mcp::McpTurnContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Teammate {
    pub name: String,
    pub title: String,
}

/// Models only know their training cutoff, so every turn is told the real date and how to use it.
pub fn current_date_block(now: chrono::DateTime<chrono::FixedOffset>) -> String {
    format!(
        "\n\n[Current date]\n\
        Right now it is {} (UTC{}). Your training data is older than this: trust this date over your own sense of what year it is.\n\
        Before saying an event, offer, deadline or opening is happening now or soon, check its dates against today. \
        Websites often keep featuring events that are already over, so read the dates on the page; if they are missing, say you could not confirm them.",
        now.format("%A, %-d %B %Y, %H:%M"),
        now.format("%:z"),
    )
}

impl TurnInput {
    pub fn format_system_prompt(&self) -> String {
        let mut base = format!(
            "{}\n\nYou are {}, {}.",
            self.kwant.persona_prompt, self.kwant.name, self.kwant.title
        );
        let soul = self.kwant.soul.trim();
        if !soul.is_empty() {
            base.push_str(&format!(
                "\n\nYour soul (let it shape your voice and judgment, but never at the cost of being accurate and helpful):\n{}",
                soul
            ));
        }
        base.push_str(&current_date_block(chrono::Local::now().fixed_offset()));
        let team_block = if let Some(ref team) = self.team_context {
            let desc = team.description.as_deref().unwrap_or("General team objectives");
            let mut ws_info = String::new();
            if let Some(ref ws) = team.workspace_path {
                ws_info.push_str(&format!("\nWorkspace: {}", ws));
            } else if let Some(ref ws) = self.workspace_dir {
                ws_info.push_str(&format!("\nWorkspace: {}", ws.display()));
            }
            if let Some(ref repo) = team.repo_url {
                ws_info.push_str(&format!("\nGit Repo: {}", repo));
            }

            format!(
                "\n\n[Active Team Alignment: @{}]{}\nTeam Mission: {}\nAs a member of team @{}, align your tone, recommendations, and analysis with this team's mission and collaborate effectively with fellow team members.",
                team.name, ws_info, desc, team.name
            )
        } else {
            "\n\nRespond clearly in character.".to_string()
        };

        let sandbox_target = self
            .sandbox_id
            .clone()
            .or_else(|| self.team_context.as_ref().map(|t| t.id.clone()))
            .unwrap_or_else(|| self.kwant.id.clone());
        let api = self.browser_api_url.clone().unwrap_or_else(|| {
            format!("http://127.0.0.1:{}/api/sandbox/{}", crate::config::DEFAULT_PORT, sandbox_target)
        });
        let sandbox_rules = format!(
            "\n\n[Bot Browser]\n\
            Your browser session is displayed live in the user's right panel; the user can browse in it too. Shell execution uses a separate container.\n\
            Browser API: {api}\n\
            Never open a host desktop browser (no `open <url>`, `xdg-open`, etc.). Use ONLY these commands (the full API, do not search for more):\n\
            - Open a page: curl -sS -X POST '{api}/navigate' -H 'Content-Type: application/json' -d '{{\"url\":\"<url>\"}}'\n\
            - Read the page (url, title, text, clickable labels): curl -sS '{api}/page'\n\
            - Click a button/link/field by its visible label: curl -sS -X POST '{api}/act' -H 'Content-Type: application/json' -d '{{\"action\":\"click_text\",\"text\":\"<label>\"}}'\n\
            - Type into the focused field: curl -sS -X POST '{api}/act' -H 'Content-Type: application/json' -d '{{\"action\":\"type\",\"text\":\"<text>\"}}'\n\
            - Press a key: curl -sS -X POST '{api}/act' -H 'Content-Type: application/json' -d '{{\"action\":\"key\",\"key\":\"Enter\"}}'\n\
            - Scroll: curl -sS -X POST '{api}/act' -H 'Content-Type: application/json' -d '{{\"action\":\"scroll\",\"delta_y\":600}}'\n\
            Every /act call returns the updated page (url, title, text, clickable), so you rarely need a separate read. \
            The current page state and its clickable labels are usually already in the request; act on them directly.\n\
            A 409 means the user has taken control: stop browser actions and wait for the user to return control. Never bypass the control API or connect to CDP directly.\n\
            Report API errors honestly. Never claim a page opened unless navigation succeeded, and only describe content that appears in the returned page text. If a response has \"blocked\": true (CAPTCHA / bot check), follow its \"notice\" instead of claiming results.\n\n\
            [Speed]\n\
            The user is waiting in a live chat. Respond as fast as possible: answer directly when you can, use the fewest tool calls (usually one), \
            do not explore the filesystem, read source code or plan multi-step investigations to figure out how the browser works, and keep replies short.",
            api = api,
        );

        let teammates_block = if self.teammates.is_empty() {
            String::new()
        } else {
            let list = self
                .teammates
                .iter()
                .map(|t| format!("- @{} ({})", t.name, t.title))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "\n\n[Teammates in this chat]\n{list}\n\
                These are the only teammates you have. To involve one, mention them as @Name in your reply, or @everyone for all of them \
                (e.g. \"@{first}, can you take the database part?\"). kwnt hands each mentioned teammate the turn right after you, \
                and they answer here in the same chat. Only use @ when you actually want that teammate to reply now; \
                to merely refer to a teammate, write their name without the @. \
                Never contact anyone else: no other agents, sessions, chats or people through tools, CLIs or APIs. \
                Never claim you asked or will relay something from a teammate unless you @mentioned them in this reply.",
                list = list,
                first = self.teammates[0].name,
            )
        };

        format!("{}{}{}{}", base, team_block, teammates_block, sandbox_rules)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub capability_desc: String,
    pub is_cli: bool,
}

/// Ollama server root, whether settings/env give the root, its `/v1` API or the full chat URL.
pub fn ollama_root(configured: Option<&str>) -> String {
    let url = configured
        .map(str::to_string)
        .or_else(|| std::env::var("OLLAMA_URL").ok())
        .unwrap_or_else(|| "http://localhost:11434".into());
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix("/chat/completions").unwrap_or(url);
    url.strip_suffix("/v1").unwrap_or(url).to_string()
}

/// Model to run for `local_ollama`: `OLLAMA_MODEL` if set, else the first one pulled.
/// `None` when the server is down or has no models.
pub async fn first_ollama_model(root: &str) -> Option<String> {
    if let Ok(m) = std::env::var("OLLAMA_MODEL") {
        if !m.trim().is_empty() {
            return Some(m.trim().to_string());
        }
    }
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_millis(500)).build().ok()?;
    let tags: serde_json::Value = client.get(format!("{root}/api/tags")).send().await.ok()?.json().await.ok()?;
    tags["models"].as_array()?.first()?["name"].as_str().map(str::to_string)
}

/// Automatically detects all currently available and functional models on the host
pub async fn detect_available_models(db: &crate::db::Database) -> Vec<ModelDescriptor> {
    let mut available = Vec::new();

    // 1. Claude
    let claude_cli = find_cli_binary("claude");
    let claude_key = db.get_setting("anthropic_api_key").await.ok().flatten()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
    if claude_cli.is_some() || claude_key.is_some() {
        available.push(ModelDescriptor {
            id: "claude".into(),
            display_name: "Claude 3.5 Sonnet".into(),
            capability_desc: if claude_key.is_none() {
                "fastest local replies (~3s): everyday questions, quick answers, plus deep architecture and complex analysis"
            } else {
                "deep architecture, multi-file refactoring, long complex analysis"
            }
            .into(),
            is_cli: claude_key.is_none() && claude_cli.is_some(),
        });
    }

    // 2. Grok
    let grok_cli = find_cli_binary("grok");
    let grok_key = db.get_setting("xai_api_key").await.ok().flatten()
        .or_else(|| std::env::var("XAI_API_KEY").ok());
    if grok_cli.is_some() || grok_key.is_some() {
        available.push(ModelDescriptor {
            id: "grok".into(),
            display_name: "xAI Grok-2".into(),
            capability_desc: if grok_key.is_none() {
                "witty banter and technical reasoning; slow local CLI (10s+ per reply), avoid for quick questions"
            } else {
                "fast technical reasoning, witty banter, quick coding solutions"
            }
            .into(),
            is_cli: grok_key.is_none() && grok_cli.is_some(),
        });
    }

    // 3. Codex
    let codex_cli = find_cli_binary("codex");
    let codex_key = db.get_setting("openai_api_key").await.ok().flatten()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok());
    if codex_cli.is_some() || codex_key.is_some() {
        available.push(ModelDescriptor {
            id: "codex".into(),
            display_name: "OpenAI Codex".into(),
            capability_desc: "direct code generation, standard algorithms, regex, JSON transforms".into(),
            is_cli: codex_key.is_none() && codex_cli.is_some(),
        });
    }

    // 4. Local Ollama
    let ollama_url = db.get_setting("ollama_url").await.ok().flatten()
        .or_else(|| std::env::var("OLLAMA_URL").ok());
    // Having the binary isn't enough: routing here only works with the server up and a model pulled.
    if let Some(model) = first_ollama_model(&ollama_root(ollama_url.as_deref())).await {
        available.push(ModelDescriptor {
            id: "local_ollama".into(),
            display_name: format!("Ollama ({model})"),
            capability_desc: "private data, offline tasks, zero API cost".into(),
            is_cli: false,
        });
    }

    // Fallback: If host has zero configured, provide default fleet so system functions
    if available.is_empty() {
        available.push(ModelDescriptor {
            id: "grok".into(),
            display_name: "Grok (Arcade Default)".into(),
            capability_desc: "fast technical reasoning, witty banter, quick coding solutions".into(),
            is_cli: false,
        });
    }

    available
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnOutput {
    pub text: String,
    pub reasoning: Option<String>,
    pub tokens_used: usize,
    /// Set when the provider refused the turn (quota, auth) before the model did anything; the
    /// driver then emitted no text and no `TurnComplete`, so the turn can move to another profile.
    #[serde(skip)]
    pub failure: Option<crate::profiles::ProviderFailure>,
}

impl TurnOutput {
    /// A turn the provider refused before anything visible happened.
    pub fn refused(failure: crate::profiles::ProviderFailure) -> Self {
        Self { failure: Some(failure), ..Default::default() }
    }
}

#[async_trait]
pub trait Driver: Send + Sync {
    async fn stream_turn(
        &self,
        input: TurnInput,
        event_tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<TurnOutput, DriverError>;
}

/// Creates a driver for a Kwant based on target model, configured API keys, and locally installed CLIs
pub fn resolve_driver_for_model(
    target_model: &str,
    configured_key: Option<String>,
    endpoint: Option<String>,
) -> Box<dyn Driver> {
    // 1. If an explicit API key is provided, use ApiDriver directly
    if let Some(key) = configured_key {
        return Box::new(ApiDriver::new(Some(key), endpoint));
    }

    // 2. Check environment variable for direct API key
    let env_key = if target_model.contains("claude") {
        std::env::var("ANTHROPIC_API_KEY").ok()
    } else if target_model.contains("grok") {
        std::env::var("XAI_API_KEY").ok()
    } else if target_model.contains("codex") || target_model.contains("gpt") {
        std::env::var("OPENAI_API_KEY").ok()
    } else {
        None
    };

    if let Some(key) = env_key {
        return Box::new(ApiDriver::new(Some(key), endpoint));
    }

    // 3. Bring-Your-Own-Agent CLI auto-discovery (Claude, Grok, Codex)
    if target_model.contains("claude") {
        if let Some(bin) = find_cli_binary("claude") {
            return Box::new(CliDriver::new(Some(bin), Some("claude".into())));
        }
    } else if target_model.contains("grok") {
        if let Some(bin) = find_cli_binary("grok") {
            return Box::new(CliDriver::new(Some(bin), Some("grok".into())));
        }
    } else if target_model.contains("codex") || target_model.contains("gpt") {
        if let Some(bin) = find_cli_binary("codex") {
            return Box::new(CliDriver::new(Some(bin), Some("codex".into())));
        }
    } else if target_model == "local_ollama" {
        return Box::new(ApiDriver::new(None, endpoint));
    }

    // 4. Fallback: check if ANY local agent CLI is installed on this host
    if let Some(bin) = find_cli_binary("claude") {
        return Box::new(CliDriver::new(Some(bin), Some("claude".into())));
    }
    if let Some(bin) = find_cli_binary("grok") {
        return Box::new(CliDriver::new(Some(bin), Some("grok".into())));
    }
    if let Some(bin) = find_cli_binary("codex") {
        return Box::new(CliDriver::new(Some(bin), Some("codex".into())));
    }

    // 5. Default to ApiDriver (which will prompt user to set an API key in Settings)
    Box::new(ApiDriver::new(None, endpoint))
}

pub fn create_driver_for_kwant(kwant: &Kwant) -> Box<dyn Driver> {
    resolve_driver_for_model(&kwant.preferred_model, None, None)
}
