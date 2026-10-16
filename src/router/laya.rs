use super::{RouteDecision, RouterError};
use reqwest::Client;
use serde_json::json;
use super::sidecar::{LayaState, LayaStatus};
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LayaRouter {
    pub base_url: String,
    pub(crate) client: Client,
    status: Arc<RwLock<LayaStatus>>,
}

impl LayaRouter {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap_or_else(|_| Client::new());

        let base_url = normalize_base_url(&base_url);
        let status = Arc::new(RwLock::new(LayaStatus::new(&base_url)));
        Self { base_url, client, status }
    }

    pub fn status(&self) -> LayaStatus {
        self.status.read().map(|s| s.clone()).unwrap_or_else(|_| LayaStatus::new(&self.base_url))
    }

    pub fn set_status(&self, state: LayaState, detail: &str, managed: bool) {
        if let Ok(mut s) = self.status.write() {
            s.state = state;
            s.detail = detail.to_string();
            s.managed = managed;
        }
    }

    /// Short note appended to routing reasons when Laya could not be asked.
    pub fn offline_note(&self) -> &'static str {
        match self.status().state {
            LayaState::Installing => "Laya is still installing, so this was a keyword match",
            LayaState::Starting => "Laya is still starting, so this was a keyword match",
            _ => "Laya is offline, so this was a keyword match",
        }
    }

    /// Returns true when the Laya sidecar answers on its health endpoint.
    pub async fn is_reachable(&self) -> bool {
        self.client
            .get(format!("{}/health", self.base_url))
            .timeout(Duration::from_millis(800))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Routes a prompt to the optimal model using Laya (/v1/systemone),
    /// falling back gracefully to local heuristics if Laya is unreachable.
    pub async fn route_prompt(&self, prompt: &str, context: &str) -> Result<RouteDecision, RouterError> {
        self.route_prompt_with_models(prompt, context, &[]).await
    }

    /// Routes a prompt dynamically choosing from the detected available models and determining needed effort.
    pub async fn route_prompt_with_models(
        &self,
        prompt: &str,
        context: &str,
        available: &[crate::drivers::ModelDescriptor],
    ) -> Result<RouteDecision, RouterError> {
        let mut criteria = serde_json::Map::new();
        let candidate_models: Vec<String> = if available.is_empty() {
            vec!["grok".into(), "claude".into(), "codex".into(), "local_ollama".into()]
        } else {
            available.iter().map(|m| m.id.clone()).collect()
        };

        if available.is_empty() {
            criteria.insert("grok".into(), serde_json::json!("fast technical reasoning, witty banter, quick coding solutions"));
            criteria.insert("claude".into(), serde_json::json!("deep architecture, multi-file refactoring, long complex analysis"));
            criteria.insert("codex".into(), serde_json::json!("direct code generation, standard algorithms, regex, JSON transforms"));
            criteria.insert("local_ollama".into(), serde_json::json!("private data, offline tasks, zero API cost"));
        } else {
            for m in available {
                criteria.insert(m.id.clone(), serde_json::Value::String(m.capability_desc.clone()));
            }
        }

        let payload = json!({
            "state": {
                "prompt": prompt,
                "context": context
            },
            "questions": {
                "model_selection": {
                    "type": "choice",
                    "instructions": "Which available model is best suited for this task?",
                    "criteria": criteria
                },
                "effort_level": {
                    "type": "choice",
                    "instructions": "What reasoning effort / compute depth is needed for this task?",
                    "criteria": {
                        "low": "quick answers, greetings, short definitions, simple edits",
                        "medium": "standard coding, explanations, moderate feature implementation",
                        "high": "deep architecture, difficult bug root cause analysis, complex refactoring"
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
                        Ok(crate::router::fallback::heuristic_fallback_with_models(prompt, context, &candidate_models))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to parse Laya response body, using fallback");
                        Ok(crate::router::fallback::heuristic_fallback_with_models(prompt, context, &candidate_models))
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "Laya service returned error, using fallback");
                Ok(crate::router::fallback::heuristic_fallback_with_models(prompt, context, &candidate_models))
            }
            Err(e) => {
                tracing::debug!(error = %e, "Laya service unreachable, using fallback");
                Ok(crate::router::fallback::heuristic_fallback_with_models(prompt, context, &candidate_models))
            }
        }
    }
}

/// Accepts either the Laya root (`http://host:8000`) or a full endpoint URL
/// (`http://host:8000/v1/systemone`) and returns the root, so paths are never doubled.
pub fn normalize_base_url(url: &str) -> String {
    let mut base = url.trim().trim_end_matches('/');
    for suffix in ["/v1/systemone/batch", "/v1/systemone", "/v1"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/');
            break;
        }
    }
    base.to_string()
}
