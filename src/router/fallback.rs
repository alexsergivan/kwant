use super::RouteDecision;

/// Phrases that reliably mean a question needs careful, multi-step reasoning. Matched as whole words
/// (see [`has_phrase`]), so "deep copy" or "security guards" don't count; bare "deep" is left out
/// because it is usually part of a name ("deep copy", "deep link", "deep learning").
const DEEP_SIGNALS: &[&str] = &[
    "architecture", "architectures", "architectural", "refactor", "refactoring", "refactored", "complex",
    "complexity", "deep dive", "in-depth", "in depth", "benchmark", "benchmarking", "security", "optimize",
    "optimise", "optimized", "optimised", "optimizing", "optimising", "optimization", "optimisation",
    "investigate", "investigating", "investigation", "root cause", "root-cause", "tradeoff", "trade-off",
    "trade off", "monolith", "monolithic", "microservice", "migrate", "migrated", "migrating", "migration",
    "postmortem", "post-mortem",
];

/// Everyday phrases that contain a deep signal but ask for nothing deep.
const NOT_DEEP: &[&str] = &["security guard", "security deposit", "social security", "security camera"];

/// Words that, just before a signal, say the person doesn't want it ("don't refactor anything").
const NEGATIONS: &[&str] = &["don't", "dont", "do not", "not", "no", "never", "without"];

/// Lower-case words of `text`. Hyphens and apostrophes stay inside a word ("trade-off", "don't"), and
/// are trimmed from its ends ("'deep copy'").
pub(crate) fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .map(|w| w.trim_matches(|c| c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Where `phrase` (one or more words) starts in `words`. A word also matches its plural ("tradeoffs").
fn phrase_at(words: &[String], phrase: &str) -> Vec<usize> {
    let parts: Vec<&str> = phrase.split(' ').collect();
    if parts.is_empty() || words.len() < parts.len() {
        return Vec::new();
    }
    let same = |w: &str, p: &str| w == p || w.strip_suffix('s').is_some_and(|x| x == p) || w.strip_suffix("es").is_some_and(|x| x == p);
    (0..=words.len() - parts.len())
        .filter(|&i| parts.iter().enumerate().all(|(j, p)| same(&words[i + j], p)))
        .collect()
}

/// True when `phrase` appears in `words` as whole words.
pub(crate) fn has_phrase(words: &[String], phrase: &str) -> bool {
    !phrase_at(words, phrase).is_empty()
}

fn has_any(words: &[String], phrases: &[&str]) -> bool {
    phrases.iter().any(|p| has_phrase(words, p))
}

/// A deep signal that isn't part of an everyday phrase and isn't negated just before it.
fn has_deep_signal(text: &str) -> bool {
    let mut ws = words(text);
    for idiom in NOT_DEEP {
        let len = idiom.split(' ').count();
        for i in phrase_at(&ws, idiom) {
            for w in &mut ws[i..i + len] {
                w.clear();
            }
        }
    }
    let negated = |i: usize| {
        let before = &ws[i.saturating_sub(2)..i];
        NEGATIONS.iter().any(|n| has_phrase(before, n))
    };
    DEEP_SIGNALS.iter().any(|s| phrase_at(&ws, s).into_iter().any(|i| !negated(i)))
}

/// Chinese, Japanese and Korean text has no spaces between words.
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Roughly how many words `prompt` has: words between spaces, and about one per two CJK characters,
/// so a Chinese paragraph isn't counted as one word.
fn word_count(prompt: &str) -> usize {
    prompt
        .split_whitespace()
        .map(|w| {
            let cjk = w.chars().filter(|&c| is_cjk(c)).count();
            if cjk == 0 { 1 } else { cjk.div_ceil(2) + usize::from(w.chars().any(|c| c.is_alphanumeric() && !is_cjk(c))) }
        })
        .sum()
}

/// Asks for a one-line answer ("in one line", "just tell me…").
const BRIEF: &[&str] = &["in one line", "one-liner", "in one word", "just tell me", "quick question"];

/// Greetings, jokes and tiny one-liners never need deep reasoning.
fn is_trivial(prompt: &str, max_words: usize) -> bool {
    let lower = prompt.to_lowercase();
    let ws = words(prompt);
    let first = ws.first().map(String::as_str).unwrap_or("");
    word_count(prompt) <= max_words
        || matches!(first, "hi" | "hello" | "hey" | "ping" | "thanks" | "thank" | "yo")
        || lower.starts_with("say ")
        || lower.contains("joke")
        || lower.contains("who are you")
        || has_any(&ws, BRIEF)
}

/// Keyword-only effort estimate, used when Laya is offline.
pub fn heuristic_effort(prompt: &str, context: &str) -> &'static str {
    if has_deep_signal(prompt) || has_deep_signal(context) || word_count(prompt) >= 30 {
        "high"
    } else if is_trivial(prompt, 6) {
        "low"
    } else {
        "medium"
    }
}

/// Laya's effort answer is a weak signal: it rated a multi-file refactor, a root-cause hunt and an
/// architecture design "medium", so a clear keyword raises those to "high". But when Laya says "low"
/// it is usually right about a keyword that is only mentioned ("don't refactor anything", "what is a
/// deep copy, in one line"), so keywords never raise a "low". Greetings are always "low".
pub fn blend_effort(prompt: &str, laya_effort: &str) -> String {
    if laya_effort != "low" && has_deep_signal(prompt) {
        "high".into()
    } else if is_trivial(prompt, 3) {
        "low".into()
    } else {
        laya_effort.into()
    }
}

/// Asks to keep the work on this machine. Bare "local" is not one of them: "localhost", "locale" and
/// "my local coffee shop" aren't private.
const PRIVATE: &[&str] = &[
    "private", "confidential", "sensitive", "offline", "ollama", "local model", "local llm", "locally",
    "on-device", "on device", "on my machine", "private data",
];

/// Needs long reasoning or writing.
const REASONING: &[&str] = &[
    "architecture", "architectural", "design doc", "tradeoff", "trade-off", "refactor", "refactoring",
    "analysis", "philosophy", "essay",
];

/// Asks for code.
const CODING: &[&str] = &[
    "python", "rust", "javascript", "typescript", "code", "script", "regex", "algorithm", "function",
];

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
    let combined = words(&format!("{prompt} {context}"));

    let effort_level = heuristic_effort(prompt, context).to_string();

    let is_available = |model: &str| -> bool {
        available_models.is_empty() || available_models.iter().any(|m| m == model)
    };

    // 1. Private / local check
    if has_any(&combined, PRIVATE)
        && is_available("local_ollama") {
            return RouteDecision {
                selected_model: "local_ollama".into(),
                effort_level: effort_level.clone(),
                confidence: 0.85,
                provider_endpoint: Some("http://localhost:11434/v1".into()),
                is_fallback: true,
                tier: None,
                model: None,
                reasoning: format!("Local privacy keywords detected; effort={}", effort_level),
            };
        }

    // 2. Complex architecture / multi-file / deep analysis check
    if has_any(&combined, REASONING)
        && is_available("claude") {
            return RouteDecision {
                selected_model: "claude".into(),
                effort_level: effort_level.clone(),
                confidence: 0.88,
                provider_endpoint: None,
                is_fallback: true,
                tier: None,
                model: None,
                reasoning: format!("Architectural reasoning keywords detected; effort={}", effort_level),
            };
        }

    // 3. Direct coding / script / regex check
    if has_any(&combined, CODING)
        && is_available("codex") {
            return RouteDecision {
                selected_model: "codex".into(),
                effort_level: effort_level.clone(),
                confidence: 0.82,
                provider_endpoint: None,
                is_fallback: true,
                tier: None,
                model: None,
                reasoning: format!("Programming keywords detected; effort={}", effort_level),
            };
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
        tier: None,
        model: None,
        reasoning: format!("Optimal available model '{}' selected with effort={}", selected, effort_level),
    }
}
