pub mod fallback;
pub mod laya;

pub use fallback::heuristic_fallback;
pub use laya::LayaRouter;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Network error communicating with model router: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Failed to parse routing decision: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteDecision {
    pub selected_model: String,
    pub confidence: f32,
    pub provider_endpoint: Option<String>,
    pub is_fallback: bool,
    pub reasoning: String,
}

/// Parses a successful response from Laya /v1/systemone into a RouteDecision
pub fn parse_laya_response(data: &serde_json::Value) -> Option<RouteDecision> {
    let answers = data.get("answers")?;
    let model_selection = answers.get("model_selection")?;
    let choice = model_selection.get("choice")?.as_str()?;
    let confidence = model_selection
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(0.9);

    let provider_endpoint = if choice == "local_ollama" {
        Some("http://localhost:11434/v1".to_string())
    } else {
        None
    };

    Some(RouteDecision {
        selected_model: choice.to_string(),
        confidence,
        provider_endpoint,
        is_fallback: false,
        reasoning: format!("Laya decision router selected {}", choice),
    })
}
