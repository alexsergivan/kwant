use super::{fallback::heuristic_fallback, RouteDecision, RouterError};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LayaRouter {
    pub base_url: String,
    client: Client,
}

impl LayaRouter {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { base_url, client }
    }

    /// Routes a prompt to the optimal model using Laya (/v1/systemone),
    /// falling back gracefully to local heuristics if Laya is unreachable.
    pub async fn route_prompt(&self, prompt: &str, context: &str) -> Result<RouteDecision, RouterError> {
        let payload = json!({
            "state": {
                "prompt": prompt,
                "context": context
            },
            "questions": {
                "model_selection": {
                    "type": "choice",
                    "instructions": "Which model is best suited for this task?",
                    "criteria": {
                        "grok": "fast technical reasoning, witty banter, quick coding solutions",
                        "claude": "deep architecture, multi-file refactoring, long complex analysis",
                        "codex": "direct code generation, standard algorithms, regex, JSON transforms",
                        "local_ollama": "private data, offline tasks, zero API cost"
                    }
                }
            }
        });

        let endpoint = format!("{}/v1/systemone", self.base_url.trim_end_matches('/'));

        match self.client.post(&endpoint).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        if let Some(decision) = super::parse_laya_response(&data) {
                            return Ok(decision);
                        }
                        tracing::warn!("Laya response missing expected fields, using fallback");
                        Ok(heuristic_fallback(prompt, context))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to parse Laya response body, using fallback");
                        Ok(heuristic_fallback(prompt, context))
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "Laya service returned error, using fallback");
                Ok(heuristic_fallback(prompt, context))
            }
            Err(e) => {
                tracing::debug!(error = %e, "Laya service unreachable, using fallback");
                Ok(heuristic_fallback(prompt, context))
            }
        }
    }
}
