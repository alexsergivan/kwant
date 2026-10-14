use super::RouteDecision;

/// Evaluates a prompt using local keyword scoring heuristics when Laya is offline.
pub fn heuristic_fallback(prompt: &str, context: &str) -> RouteDecision {
    heuristic_fallback_with_models(prompt, context, &[])
}

/// Evaluates prompt and selects best model from available candidates along with required effort level.
pub fn heuristic_fallback_with_models(
    prompt: &str,
    context: &str,
    available_models: &[String],
) -> RouteDecision {
    let lower_prompt = prompt.to_lowercase();
    let lower_context = context.to_lowercase();
    let combined = format!("{} {}", lower_prompt, lower_context);
    let word_count = prompt.split_whitespace().count();

    // Determine effort level: low, medium, high
    let is_high_effort = combined.contains("architecture")
        || combined.contains("refactor")
        || combined.contains("complex")
        || combined.contains("deep")
        || combined.contains("benchmark")
        || combined.contains("security")
        || combined.contains("optimize")
        || combined.contains("investigate")
        || word_count >= 30;

    let is_low_effort = word_count <= 6
        || lower_prompt.starts_with("hi")
        || lower_prompt.starts_with("hello")
        || lower_prompt.starts_with("hey")
        || lower_prompt.starts_with("say ")
        || lower_prompt.starts_with("ping")
        || lower_prompt.contains("joke")
        || lower_prompt.contains("who are you");

    let effort_level = if is_high_effort {
        "high".to_string()
    } else if is_low_effort {
        "low".to_string()
    } else {
        "medium".to_string()
    };

    let is_available = |model: &str| -> bool {
        available_models.is_empty() || available_models.iter().any(|m| m == model)
    };

    // 1. Private / local check
    if combined.contains("private")
        || combined.contains("confidential")
        || combined.contains("local")
        || combined.contains("offline")
        || combined.contains("sensitive")
    {
        if is_available("local_ollama") {
            return RouteDecision {
                selected_model: "local_ollama".into(),
                effort_level: effort_level.clone(),
                confidence: 0.85,
                provider_endpoint: Some("http://localhost:11434/v1".into()),
                is_fallback: true,
                reasoning: format!("Local privacy keywords detected; effort={}", effort_level),
            };
        }
    }

    // 2. Complex architecture / multi-file / deep analysis check
    if combined.contains("architecture")
        || combined.contains("design doc")
        || combined.contains("tradeoff")
        || combined.contains("refactor")
        || combined.contains("analysis")
        || combined.contains("philosophy")
        || combined.contains("essay")
    {
        if is_available("claude") {
            return RouteDecision {
                selected_model: "claude".into(),
                effort_level: effort_level.clone(),
                confidence: 0.88,
                provider_endpoint: None,
                is_fallback: true,
                reasoning: format!("Architectural reasoning keywords detected; effort={}", effort_level),
            };
        }
    }

    // 3. Direct coding / script / regex check
    if combined.contains("python")
        || combined.contains("rust")
        || combined.contains("javascript")
        || combined.contains("typescript")
        || combined.contains("code")
        || combined.contains("script")
        || combined.contains("regex")
        || combined.contains("algorithm")
        || combined.contains("function")
    {
        if is_available("codex") {
            return RouteDecision {
                selected_model: "codex".into(),
                effort_level: effort_level.clone(),
                confidence: 0.82,
                provider_endpoint: None,
                is_fallback: true,
                reasoning: format!("Programming keywords detected; effort={}", effort_level),
            };
        }
    }

    // 4. Default fast & general
    let selected = if is_available("grok") {
        "grok"
    } else if is_available("claude") {
        "claude"
    } else if is_available("codex") {
        "codex"
    } else if !available_models.is_empty() {
        &available_models[0]
    } else {
        "grok"
    };

    RouteDecision {
        selected_model: selected.into(),
        effort_level: effort_level.clone(),
        confidence: 0.75,
        provider_endpoint: None,
        is_fallback: true,
        reasoning: format!("Optimal available model '{}' selected with effort={}", selected, effort_level),
    }
}
