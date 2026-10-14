pub mod api;
pub mod cli;
pub mod events;

pub use api::ApiDriver;
pub use cli::CliDriver;
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

/// Factory function creating the appropriate driver for a given Kwant
pub fn create_driver_for_kwant(kwant: &Kwant) -> Box<dyn Driver> {
    if kwant.driver_type == "cli" {
        Box::new(CliDriver::new(Some("claude".into())))
    } else {
        Box::new(ApiDriver::new(None, None))
    }
}
