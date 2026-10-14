pub mod api;
pub mod cli;
pub mod events;

pub use api::ApiDriver;
pub use cli::{find_cli_binary, CliDriver};
pub use events::RuntimeEvent;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInput {
    pub thread_id: String,
    pub kwant: Kwant,
    pub prompt: String,
    pub history: Vec<Message>,
    pub model_override: Option<String>,
    pub effort_level: Option<String>,
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
            capability_desc: "deep architecture, multi-file refactoring, long complex analysis".into(),
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
            capability_desc: "fast technical reasoning, witty banter, quick coding solutions".into(),
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
