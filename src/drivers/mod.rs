pub mod api;
pub mod cli;
pub mod events;

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
    /// Sandbox whose browser is shown to the user for this conversation (team, channel or kwant).
    #[serde(default)]
    pub sandbox_id: Option<String>,
}

static API_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// Records the port the HTTP server listens on, so agents get working sandbox API URLs.
pub fn set_api_port(port: u16) {
    let _ = API_PORT.set(port);
}

fn api_port() -> u16 {
    *API_PORT.get().unwrap_or(&8899)
}

impl TurnInput {
    pub fn format_system_prompt(&self) -> String {
        let base = format!(
            "{}\n\nYou are {}, {}.",
            self.kwant.persona_prompt, self.kwant.name, self.kwant.title
        );
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

        let api = format!("http://127.0.0.1:{}/api/sandbox/{}", api_port(), sandbox_target);
        let sandbox_rules = format!(
            "\n\n[Virtual Machine Sandbox & Right-Panel In-Container Browser]\n\
            You have an isolated Sandbox Virtual Machine with a headless Chromium browser displayed live in the user's right-side UI panel. The user can browse in it too.\n\
            Sandbox ID: {sandbox}\n\
            CRITICAL BROWSING INSTRUCTIONS:\n\
            1. NEVER execute host desktop commands (e.g. `open <url>`, `start <url>`, `xdg-open`) or attempt to launch browser windows on the user's desktop!\n\
            2. All web browsing must happen inside the right-side Sandbox browser, using ONLY these commands (the full API, do not search for more):\n\
               - Open a page: curl -s -X POST {api}/navigate -H 'Content-Type: application/json' -d '{{\"url\":\"<url>\"}}'\n\
               - Read the page (url, title, text, clickable labels): curl -s {api}/page\n\
               - Click a button/link/field by its visible label: curl -s -X POST {api}/act -H 'Content-Type: application/json' -d '{{\"action\":\"click_text\",\"text\":\"<label>\"}}'\n\
               - Type into the focused field: curl -s -X POST {api}/act -H 'Content-Type: application/json' -d '{{\"action\":\"type\",\"text\":\"<text>\"}}'\n\
               - Press a key: curl -s -X POST {api}/act -H 'Content-Type: application/json' -d '{{\"action\":\"key\",\"key\":\"Enter\"}}'\n\
               - Scroll: curl -s -X POST {api}/act -H 'Content-Type: application/json' -d '{{\"action\":\"scroll\",\"delta_y\":600}}'\n\
               Every /act call returns the updated page (url, title, text, clickable), so you rarely need a separate read.\n\
            3. The current page state and its clickable labels are usually already in the request. Act on them directly.\n\n\
            [Speed]\n\
            The user is waiting in a live chat. Respond as fast as possible: answer directly when you can, use the fewest tool calls (usually one), \
            do not explore the filesystem, read source code or plan multi-step investigations to figure out how the browser works, and keep replies short.",
            sandbox = sandbox_target,
            api = api,
        );

        format!("{}{}{}", base, team_block, sandbox_rules)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub capability_desc: String,
    pub is_cli: bool,
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
    let ollama_cli = find_cli_binary("ollama");
    if ollama_url.is_some() || ollama_cli.is_some() {
        available.push(ModelDescriptor {
            id: "local_ollama".into(),
            display_name: "Local Ollama".into(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnOutput {
    pub text: String,
    pub reasoning: Option<String>,
    pub tokens_used: usize,
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
        return Box::new(ApiDriver::new(None, Some("http://localhost:11434/v1".into())));
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
