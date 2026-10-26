pub mod fallback;
pub mod laya;
pub mod sidecar;
pub mod sticky;
pub mod tags;
pub mod team;

pub use fallback::heuristic_fallback;
pub use laya::LayaRouter;
pub use tags::{handoff_mentions, handoff_targets, only_credited, HandoffMentions, parse_mentions, parse_mentions_with_lookup, TagMention};
pub use team::{
    heuristic_pick_kwant, heuristic_team_decision, matched_terms, parse_laya_kwant_choice,
    parse_laya_team_answers, ChannelRoute, TeamDecision,
};
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
    pub effort_level: String, // "low", "medium", "high"
    pub confidence: f32,
    pub provider_endpoint: Option<String>,
    pub is_fallback: bool,
    pub reasoning: String,
    /// The model tier, from the effort (set by [`apply_preferred_model`]).
    #[serde(default)]
    pub tier: Option<crate::drivers::engines::Tier>,
    /// The model to run: a pinned one first, then the one picked for the engine the turn runs on.
    #[serde(default)]
    pub model: Option<String>,
}

/// A kwant's `preferred_model`, read.
///
/// - `auto` (or empty): Laya picks the engine family and the model tier.
/// - `<family>` (`claude`, `codex`, `grok`, `ollama`/`local_ollama`): Laya picks only the tier.
/// - `<family>/<model>` or `<family>:<model>` (`claude/sonnet`, `codex:gpt-6.1-sol`,
///   `ollama/llama3.2:latest`): that model, fixed.
/// - Anything else is an older pin of a model id (`claude-opus-5-5`, `gpt-5.5`): fixed to it, on
///   the family its name belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// Routing id of the family (`claude`, `codex`, `grok`, `local_ollama`); `None` = auto.
    pub family: Option<String>,
    /// A fixed model; `None` lets Laya pick the tier.
    pub model: Option<String>,
}

fn family_id(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "grok" => Some("grok"),
        "custom" => Some("custom"),
        "ollama" | "local_ollama" => Some("local_ollama"),
        _ => None,
    }
}

pub fn parse_pin(preferred_model: &str) -> Pin {
    let pinned = preferred_model.trim();
    if pinned.is_empty() || pinned.eq_ignore_ascii_case("auto") {
        return Pin { family: None, model: None };
    }
    if let Some(family) = family_id(pinned) {
        return Pin { family: Some(family.into()), model: None };
    }
    // The first separator splits, so Ollama tags like "llama3.2:latest" stay whole
    if let Some(i) = pinned.find(['/', ':']) {
        if let Some(family) = family_id(&pinned[..i]) {
            let model = pinned[i + 1..].trim();
            let model = (!model.is_empty() && !model.eq_ignore_ascii_case("auto")).then(|| model.to_string());
            return Pin { family: Some(family.into()), model };
        }
    }
    match crate::profiles::Engine::from_model(pinned) {
        Some(engine) => Pin { family: Some(engine.as_str().into()), model: Some(pinned.into()) },
        // Unknown: routing says no engine runs it
        None => Pin { family: Some(pinned.into()), model: None },
    }
}

/// A bot's pinned engine (anything other than "auto") always wins; Laya still decides the effort,
/// and with it the model tier unless the model is pinned too.
pub fn apply_preferred_model(mut route: RouteDecision, preferred_model: &str) -> RouteDecision {
    let pin = parse_pin(preferred_model);
    route.tier = Some(crate::drivers::engines::Tier::from_effort(&route.effort_level));
    let Some(family) = pin.family else {
        return route;
    };
    route.reasoning = match &pin.model {
        Some(model) => format!(
            "Model pinned to '{family}' · '{model}' in the bot's settings; effort={} (Laya suggested '{}')",
            route.effort_level, route.selected_model
        ),
        None => format!(
            "Engine pinned to '{family}' in the bot's settings; effort={} picks the tier (Laya suggested '{}')",
            route.effort_level, route.selected_model
        ),
    };
    route.selected_model = family;
    route.model = pin.model;
    route.confidence = 1.0;
    route.provider_endpoint = None;
    route
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

    let effort_level = answers
        .get("effort_level")
        .and_then(|e| e.get("choice"))
        .and_then(|c| c.as_str())
        .unwrap_or("medium")
        .to_string();

    let provider_endpoint = if choice == "local_ollama" {
        Some("http://localhost:11434/v1".to_string())
    } else {
        None
    };

    Some(RouteDecision {
        selected_model: choice.to_string(),
        effort_level: effort_level.clone(),
        confidence,
        provider_endpoint,
        is_fallback: false,
        reasoning: format!("Laya decision router selected {} with {} effort", choice, effort_level),
        tier: None,
        model: None,
    })
}
