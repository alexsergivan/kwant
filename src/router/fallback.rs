use super::RouteDecision;

/// Phrases that reliably mean a question needs careful, multi-step reasoning.
const DEEP_SIGNALS: &[&str] = &[
    "architecture", "architectural", "refactor", "complex", "deep", "benchmark", "security", "optimize",
    "investigate", "root cause", "root-cause", "tradeoff", "trade-off", "monolith", "microservice",
    "migrat", "postmortem", "post-mortem",
];

fn has_deep_signal(lower: &str) -> bool {
    DEEP_SIGNALS.iter().any(|s| lower.contains(s))
}

/// Greetings, jokes and tiny one-liners never need deep reasoning.
fn is_trivial(prompt: &str, max_words: usize) -> bool {
    let lower = prompt.to_lowercase();
    let first = lower.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()).unwrap_or("");
    prompt.split_whitespace().count() <= max_words
        || matches!(first, "hi" | "hello" | "hey" | "ping" | "thanks" | "thank" | "yo")
        || lower.starts_with("say ")
        || lower.contains("joke")
        || lower.contains("who are you")
}

/// Keyword-only effort estimate, used when Laya is offline.
pub fn heuristic_effort(prompt: &str, context: &str) -> &'static str {
    let combined = format!("{} {}", prompt.to_lowercase(), context.to_lowercase());
    if has_deep_signal(&combined) || prompt.split_whitespace().count() >= 30 {
        "high"
    } else if is_trivial(prompt, 6) {
        "low"
    } else {
        "medium"
    }
}

/// Laya's effort answer is a weak signal (it rated greetings "medium" and root-cause or trade-off
/// questions "low"), so unambiguous keyword signals override it in either direction.
pub fn blend_effort(prompt: &str, laya_effort: &str) -> String {
    if has_deep_signal(&prompt.to_lowercase()) {
        "high".into()
    } else if is_trivial(prompt, 3) {
        "low".into()
    } else {
        laya_effort.into()
    }
}

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

    let effort_level = heuristic_effort(prompt, context).to_string();

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

    // 4. Default fast & general — ordered by measured CLI latency (claude ~3s, codex ~7s, grok 10s+)
    let selected = if is_available("claude") {
        "claude"
    } else if is_available("grok") {
        "grok"
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
