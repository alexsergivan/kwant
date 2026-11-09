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
    "postmortem", "post-mortem", "deep analysis", "think deeply", "think hard", "go deep", "dig deep",
];

/// Everyday phrases that contain a deep signal but ask for nothing deep.
const NOT_DEEP: &[&str] = &["security guard", "security deposit", "social security", "security camera"];

/// Words that, just before a signal, say the person doesn't want it ("don't refactor anything").
const NEGATIONS: &[&str] = &["don't", "dont", "do not", "not", "no", "never", "without"];

/// After a negation, these turn it back into a request: "don't ignore security warnings" wants them looked at.
const UNDO_NEGATION: &[&str] = &["ignore", "skip", "forget", "overlook", "neglect", "miss", "underestimate"];

/// How far back a negation reaches: "don't worry about the refactor" is four words.
const NEGATION_WINDOW: usize = 4;

/// After a negation, these say the person is confused, not declining: "I don't understand the architecture".
const NOT_A_REFUSAL: &[&str] = &["know", "understand", "get", "see", "sure"];

/// Lower-case words of `text`. Apostrophes stay inside a word ("don't") but a possessive "'s" is
/// dropped ("Rust's" is "rust"). Hyphens split a word ("code-review" is "code review"), and keyword
/// phrases are split the same way, so "trade-off" still matches "trade-off" and "trade off".
pub(crate) fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\''))
        .map(|w| w.strip_suffix("'s").unwrap_or(w))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Where `phrase` (one or more words) starts in `words`. A word also matches its plural ("tradeoffs")
/// and a version number after it ("python3").
fn phrase_at(words: &[String], phrase: &str) -> Vec<usize> {
    let parts: Vec<&str> = phrase.split([' ', '-']).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || words.len() < parts.len() {
        return Vec::new();
    }
    let unversioned = |w: &str| {
        let stem = w.trim_end_matches(|c: char| c.is_ascii_digit());
        if stem.len() >= 4 && stem.chars().all(char::is_alphabetic) { stem.to_string() } else { w.to_string() }
    };
    let same = |w: &str, p: &str| {
        let w = unversioned(w);
        w == p || w.strip_suffix('s').is_some_and(|x| x == p) || w.strip_suffix("es").is_some_and(|x| x == p)
    };
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

/// The clauses of `text`, as [`words`]. A clause ends at punctuation or at "but"/"just", so a
/// negation in one ("don't refactor it, just…") doesn't reach into the next.
fn clauses(text: &str) -> Vec<Vec<String>> {
    text.split([',', '.', ';', ':', '!', '?', '\n'])
        .flat_map(|c| words(c).split(|w| w == "but" || w == "just").map(<[String]>::to_vec).collect::<Vec<_>>())
        .filter(|c| !c.is_empty())
        .collect()
}

/// A deep signal that isn't part of an everyday phrase and isn't negated earlier in its clause.
fn has_deep_signal(text: &str) -> bool {
    clauses(text).into_iter().any(|mut ws| {
        for idiom in NOT_DEEP {
            let len = idiom.split([' ', '-']).count();
            for i in phrase_at(&ws, idiom) {
                for w in &mut ws[i..i + len] {
                    w.clear();
                }
            }
        }
        let negated = |i: usize| {
            let before = &ws[i.saturating_sub(NEGATION_WINDOW)..i];
            let refusal = NEGATIONS.iter().any(|n| {
                phrase_at(before, n).into_iter().any(|j| {
                    let next = before.get(j + n.split(' ').count()).map(String::as_str).unwrap_or("");
                    !NOT_A_REFUSAL.contains(&next)
                })
            });
            refusal && !has_any(before, UNDO_NEGATION)
        };
        DEEP_SIGNALS.iter().any(|s| phrase_at(&ws, s).into_iter().any(|i| !negated(i)))
    })
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

/// Words of the person's own message below which an attachment alone doesn't earn the deep tier.
const SHORT_WITH_ATTACHMENT: usize = 30;

/// A short message whose weight is an attached file ("what's on this?" plus a PDF) gets at most
/// "medium" effort: reading the file is work for the tools, not a reason for the deepest model.
/// Asking for depth in so many words ("analyze this thoroughly") still keeps "high".
pub fn cap_effort_for_attachment(prompt: &str, effort: &str) -> Option<&'static str> {
    let (own, _files) = prompt.split_once("\n\n[Attached files]")?;
    (effort.eq_ignore_ascii_case("high") && word_count(own) <= SHORT_WITH_ATTACHMENT && !has_deep_signal(own)).then_some("medium")
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

/// Words that say a short message carries on with the work in hand: "show me mockup image",
/// "Screenshot give me!", "Fix differences!". On 2026-11-09 such follow-ups were rated "low" and
/// ran on the fast tier (Haiku), which described the mockup twice without attaching it.
const CONTINUES_WORK: &[&str] = &[
    "show", "give", "fix", "make", "build", "send", "screenshot", "do", "try", "again", "push", "commit",
    "deploy", "redo",
];

/// Greetings and thanks: they stay "low" even right after a turn that used tools.
const SMALL_TALK: &[&str] = &[
    "hi", "hello", "hey", "yo", "ping", "thanks", "thank", "thx", "ty", "cheers", "bye", "good morning",
    "good night", "good evening",
];

/// Up to this many words a message counts as a short follow-up. Longer ones are rated as they are.
pub const SHORT_FOLLOW_UP: usize = 12;

/// The bot's previous reply in the same chat, as the router needs it for a short follow-up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousTurn {
    /// "low", "medium" or "high": the effort (or the tier, read as one) that reply ran on.
    pub effort: Option<String>,
    /// It ran tools (shell, browser, file edits): the person is in the middle of work with it.
    pub used_tools: bool,
}

fn effort_rank(effort: &str) -> u8 {
    match effort {
        "low" => 0,
        "high" => 2,
        _ => 1,
    }
}

fn effort_of_rank(rank: u8) -> &'static str {
    match rank {
        0 => "low",
        1 => "medium",
        _ => "high",
    }
}

/// A greeting or thanks with no action word in it ("hi", "thanks!", "thank you so much").
pub fn is_small_talk(prompt: &str) -> bool {
    let ws = words(prompt);
    SMALL_TALK.iter().any(|p| phrase_at(&ws, p).first() == Some(&0)) && !has_any(&ws, CONTINUES_WORK)
}

/// A short message that carries on with the work: it has an action word, or the bot's previous
/// reply here used tools and the message isn't a greeting or thanks.
pub fn continues_work(prompt: &str, previous: Option<&PreviousTurn>) -> bool {
    if word_count(prompt) > SHORT_FOLLOW_UP {
        return false;
    }
    has_any(&words(prompt), CONTINUES_WORK) || (previous.is_some_and(|p| p.used_tools) && !is_small_talk(prompt))
}

/// The effort for a short follow-up that continues work: the previous turn's, and never below
/// "medium" (the balanced tier). `Some` only when that raises `effort`; greetings, thanks and
/// messages that start something new keep what the router said.
pub fn follow_up_effort(prompt: &str, effort: &str, previous: Option<&PreviousTurn>) -> Option<&'static str> {
    if !continues_work(prompt, previous) {
        return None;
    }
    let kept = previous.and_then(|p| p.effort.as_deref()).map_or(0, effort_rank);
    let floor = kept.max(effort_rank("medium"));
    (floor > effort_rank(effort)).then(|| effort_of_rank(floor))
}

/// The tier a reply ran on, from its saved `meta.ran_on`, as an effort ("fast" is "low").
fn ran_on_effort(meta: &serde_json::Value) -> Option<String> {
    let ran_on = meta.get("ran_on")?;
    let from_tier = ran_on.get("tier").and_then(|t| t.as_str()).map(|t| match t {
        "fast" => "low",
        "deep" => "high",
        _ => "medium",
    });
    from_tier.or_else(|| ran_on.get("effort").and_then(|e| e.as_str())).map(str::to_string)
}

/// `kwant`'s last reply in `recent` (the chat's newest messages, oldest first): what it ran on and
/// whether it used tools. Saved replies carry the bot's name as `sender_id`; its id matches too.
pub fn previous_turn(recent: &[crate::db::models::Message], kwant_id: &str, kwant_name: &str) -> Option<PreviousTurn> {
    let m = recent
        .iter()
        .rev()
        .find(|m| m.sender_role == "kwant" && (m.sender_id == kwant_id || m.sender_id == kwant_name))?;
    let meta = m.meta.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()).unwrap_or_default();
    let steps = meta.get("activity").and_then(|a| a.get("steps")).and_then(|s| s.as_array()).is_some_and(|s| !s.is_empty());
    let calls = m.tool_calls.as_deref().is_some_and(|t| !matches!(t.trim(), "" | "null" | "[]"));
    Some(PreviousTurn { effort: ran_on_effort(&meta), used_tools: steps || calls })
}

/// Asks to keep the work on this machine. Bare "local" and "private" are not among them: "localhost",
/// "my local coffee shop", "Safari private mode" and "a private method" aren't about where it runs.
const PRIVATE: &[&str] = &[
    "confidential", "sensitive", "offline", "ollama", "local model", "local llm", "locally", "on-device",
    "on my machine", "private data", "private files", "private documents", "private info",
    "private information", "keep it private", "keep this private",
    // "my private" only before the person's own records: "my private repo" and "my private method" are code
    "my private file", "my private document", "my private note", "my private diary", "my private record",
    "my private health", "my private medical",
];

/// Needs long reasoning or writing.
const REASONING: &[&str] = &[
    "architecture", "architectural", "design doc", "tradeoff", "trade-off", "refactor", "refactoring",
    "analysis", "philosophy", "essay",
];

/// Asks for code.
const CODING: &[&str] = &[
    "python", "rust", "javascript", "typescript", "code", "codebase", "coding", "script", "scripting",
    "regex", "algorithm", "function",
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

#[cfg(test)]
mod follow_up_tests {
    use super::*;
    use crate::drivers::engines::Tier;

    /// The person's follow-ups from the 2026-11-09 chat, verbatim.
    const HIS_FOLLOW_UPS: &[&str] = &[
        "show me mockup image",
        "Screenshot give me!",
        "Show mw screenshot in chat!!",
        "Fix differences!",
        "show me how it lloks on page",
    ];

    /// The tier the turn runs on after Laya's (or the keywords') effort and the follow-up floor.
    fn tier(prompt: &str, laya: &str, previous: Option<&PreviousTurn>) -> Tier {
        let blended = blend_effort(prompt, laya);
        let effort = follow_up_effort(prompt, &blended, previous).map_or(blended, str::to_string);
        Tier::from_effort(&effort)
    }

    fn after_tools(effort: &str) -> PreviousTurn {
        PreviousTurn { effort: Some(effort.into()), used_tools: true }
    }

    #[test]
    fn his_follow_ups_route_to_at_least_balanced() {
        let fresh = None;
        let tools_low = after_tools("low");
        let tools_medium = after_tools("medium");
        let words_only = PreviousTurn { effort: Some("medium".into()), used_tools: false };
        for msg in HIS_FOLLOW_UPS {
            for laya in ["low", "medium"] {
                for previous in [fresh, Some(&tools_low), Some(&tools_medium), Some(&words_only)] {
                    assert_ne!(tier(msg, laya, previous), Tier::Fast, "{msg:?} (Laya {laya}, previous {previous:?}) dropped to the fast tier");
                }
                // The keyword fallback (Laya offline) rates them "low" too; the floor lifts them
                assert_ne!(tier(msg, heuristic_effort(msg, "chat"), fresh), Tier::Fast, "{msg:?} on the keyword fallback");
            }
        }
        // Before this fix they were rated "low" (the bug): the floor is what lifts them
        assert_eq!(blend_effort("Screenshot give me!", "medium"), "low");
        assert_eq!(blend_effort("Fix differences!", "medium"), "low");
    }

    #[test]
    fn a_follow_up_keeps_the_previous_turns_tier() {
        let deep = after_tools("high");
        for msg in HIS_FOLLOW_UPS {
            assert_eq!(tier(msg, "low", Some(&deep)), Tier::Deep, "{msg:?} after a deep turn");
            assert_eq!(tier(msg, "medium", Some(&deep)), Tier::Deep);
        }
        // Never lowers what the router said
        assert_eq!(follow_up_effort("Fix differences!", "high", Some(&after_tools("low"))), None);
        // A Haiku turn before doesn't pull the next one down: the floor is balanced
        assert_eq!(tier("Fix differences!", "low", Some(&after_tools("low"))), Tier::Balanced);
    }

    #[test]
    fn greetings_and_thanks_stay_low() {
        let tools = after_tools("high");
        for msg in ["hi", "thanks", "Thanks!", "thank you", "hey", "hello there"] {
            assert_eq!(tier(msg, "low", None), Tier::Fast, "{msg:?}");
            assert_eq!(tier(msg, "low", Some(&tools)), Tier::Fast, "{msg:?} right after a turn with tools");
            assert!(is_small_talk(msg), "{msg:?}");
        }
        // Thanks with an action in it carries on the work
        assert!(!is_small_talk("thanks, now push it"));
        assert_eq!(tier("thanks, now push it", "low", None), Tier::Balanced);
    }

    #[test]
    fn a_short_reply_after_a_turn_with_tools_continues_it() {
        // No action word, but the bot was working: "and the dark one?" is part of that work
        assert_eq!(tier("and the dark one?", "low", Some(&after_tools("medium"))), Tier::Balanced);
        // Without tools before (a plain chat) a tiny message stays where the router put it
        let chat = PreviousTurn { effort: Some("medium".into()), used_tools: false };
        assert_eq!(tier("and the dark one?", "low", Some(&chat)), Tier::Fast);
        assert_eq!(tier("what is 2+2?", "low", None), Tier::Fast);
        // A long message is rated as it is
        let long = "please tell me in a few words what the weather is usually like in Lisbon in November and why";
        assert!(!continues_work(long, Some(&after_tools("medium"))));
    }

    fn msg(role: &str, sender: &str, meta: Option<serde_json::Value>, tool_calls: Option<&str>) -> crate::db::models::Message {
        crate::db::models::Message {
            id: uuid::Uuid::new_v4().to_string(),
            channel_id: Some("c".into()),
            direct_kwant_id: None,
            sender_role: role.into(),
            sender_id: sender.into(),
            text: "x".into(),
            reasoning_content: None,
            tool_calls: tool_calls.map(str::to_string),
            tokens_used: 0,
            created_at: None,
            thread_id: None,
            meta: meta.map(|m| m.to_string()),
            results: None,
            reply_to_message_id: None,
            reaction: None,
            reply_to: None,
        }
    }

    #[test]
    fn previous_turn_reads_the_bots_last_reply() {
        let worked = serde_json::json!({"ran_on": {"tier": "deep", "effort": "high"}, "activity": {"steps": [{"title": "Running `ls`"}]}});
        let talked = serde_json::json!({"ran_on": {"tier": "fast", "effort": "low"}});
        let recent = vec![
            msg("user", "person", None, None),
            msg("kwant", "Frontend", Some(worked), None),
            msg("kwant", "growth-id", Some(talked.clone()), None),
            msg("user", "person", None, None),
        ];
        // Saved replies carry the bot's name as sender; its id works too
        assert_eq!(previous_turn(&recent, "frontend-id", "Frontend"), Some(PreviousTurn { effort: Some("high".into()), used_tools: true }));
        assert_eq!(previous_turn(&recent, "growth-id", "Growth"), Some(PreviousTurn { effort: Some("low".into()), used_tools: false }));
        assert_eq!(previous_turn(&recent, "other", "Other"), None);
        // MCP tool records count as tools too
        let mcp = vec![msg("kwant", "Frontend", Some(talked), Some(r#"[{"name":"browser_screenshot"}]"#))];
        assert!(previous_turn(&mcp, "f", "Frontend").unwrap().used_tools);
    }
}
