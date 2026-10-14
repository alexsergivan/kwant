use super::RouteDecision;

/// Evaluates a prompt using local keyword scoring heuristics when Laya is offline.
pub fn heuristic_fallback(prompt: &str, context: &str) -> RouteDecision {
    let lower_prompt = prompt.to_lowercase();
    let lower_context = context.to_lowercase();
    let combined = format!("{} {}", lower_prompt, lower_context);

    // 1. Private / local check
    if combined.contains("private")
        || combined.contains("confidential")
        || combined.contains("local")
        || combined.contains("offline")
        || combined.contains("sensitive")
    {
        return RouteDecision {
            selected_model: "local_ollama".into(),
            confidence: 0.85,
            provider_endpoint: Some("http://localhost:11434/v1".into()),
            is_fallback: true,
            reasoning: "Local privacy keywords detected in prompt".into(),
        };
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
        return RouteDecision {
            selected_model: "claude".into(),
            confidence: 0.88,
            provider_endpoint: None,
            is_fallback: true,
            reasoning: "Architectural reasoning keywords detected in prompt".into(),
        };
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
        return RouteDecision {
            selected_model: "codex".into(),
            confidence: 0.82,
            provider_endpoint: None,
            is_fallback: true,
            reasoning: "Programming keywords detected in prompt".into(),
        };
    }

    // 4. Default: grok for speed, witty tone, and general queries
    RouteDecision {
        selected_model: "grok".into(),
        confidence: 0.75,
        provider_endpoint: None,
        is_fallback: true,
        reasoning: "Default fast model selected".into(),
    }
}
