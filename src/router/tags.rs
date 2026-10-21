use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagMention {
    Kwant(String),
    Team(String),
}

impl TagMention {
    pub fn name(&self) -> &str {
        match self {
            TagMention::Kwant(name) => name,
            TagMention::Team(name) => name,
        }
    }

    pub fn is_team(&self) -> bool {
        matches!(self, TagMention::Team(_))
    }

    pub fn is_kwant(&self) -> bool {
        matches!(self, TagMention::Kwant(_))
    }
}

/// Parses @mentions in text and classifies them as Kwant or Team mentions.
/// Mentions follow the pattern `@([a-zA-Z0-9_-]+)`.
/// Mentions containing hyphens or matching team naming conventions (team, squad, group, engineering, etc.)
/// are classified as Team mentions, while other single-name tokens are classified as Kwants.
pub fn parse_mentions(text: &str) -> Vec<TagMention> {
    parse_mentions_with_lookup(text, |_| None)
}

/// Parses @mentions with an optional custom lookup function before falling back to heuristics.
pub fn parse_mentions_with_lookup(
    text: &str,
    lookup: impl Fn(&str) -> Option<bool>,
) -> Vec<TagMention> {
    let mut mentions = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' {
            let is_boundary = i == 0
                || (!chars[i - 1].is_ascii_alphanumeric() && chars[i - 1] != '_' && chars[i - 1] != '-');

            if is_boundary {
                i += 1;
                let start = i;
                while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '-') {
                    i += 1;
                }
                if i > start {
                    let token: String = chars[start..i].iter().collect();
                    if let Some(is_team) = lookup(&token) {
                        if is_team {
                            mentions.push(TagMention::Team(token));
                        } else {
                            mentions.push(TagMention::Kwant(token));
                        }
                    } else if is_team_mention(&token) {
                        mentions.push(TagMention::Team(token));
                    } else {
                        mentions.push(TagMention::Kwant(token));
                    }
                    continue;
                }
            }
        }
        i += 1;
    }

    mentions
}

fn is_team_mention(token: &str) -> bool {
    if token.contains('-') {
        return true;
    }
    let lower = token.to_lowercase();
    lower.contains("team")
        || lower.contains("squad")
        || lower.contains("group")
        || lower.ends_with("engineering")
        || lower.ends_with("devops")
        || lower.ends_with("infra")
}

/// Teammates (as indices into `names`) that `text` hands the turn to with `@Name`,
/// in order of first mention. `@everyone` / `@all` selects every teammate.
/// See [`handoff_mentions`] for what counts.
pub fn handoff_targets(text: &str, names: &[&str]) -> Vec<usize> {
    handoff_mentions(text, names).targets
}

/// Words written after an `@` that address the person or the chat, not a bot.
const NOT_A_BOT: [&str; 7] = ["you", "user", "me", "here", "channel", "team", "person"];

/// What a reply's @mentions hand on, and the mentions that hand nothing on, so the caller can
/// say why instead of dropping them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HandoffMentions {
    /// Teammates handed the turn, as indices into `names`, in order of first mention.
    pub targets: Vec<usize>,
    /// Teammates mentioned only inside a ``` code block.
    pub in_code_block: Vec<usize>,
    /// `@Name`s in prose that match no teammate, as written, once each.
    pub unknown: Vec<String>,
}

/// Reads the @mentions in `text` against `names`. Prose mentions count, and so does inline
/// code that is just a teammate's name (`` `@Growth` ``: models often format names that way).
/// Other inline code, ``` code blocks and email addresses hand nothing on.
pub fn handoff_mentions(text: &str, names: &[&str]) -> HandoffMentions {
    let is_name = |tag: &str| names.iter().position(|n| n.eq_ignore_ascii_case(tag));
    let chars: Vec<char> = text.chars().collect();
    let find = |from: usize, pat: &[char]| (from..=chars.len().saturating_sub(pat.len())).find(|&j| chars[j..].starts_with(pat));
    let fence = ['`', '`', '`'];
    let mut prose = String::with_capacity(text.len());
    let mut fenced = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&fence) {
            let end = find(i + 3, &fence).unwrap_or(chars.len());
            fenced.extend(&chars[i + 3..end]);
            fenced.push('\n');
            prose.push(' ');
            i = (end + 3).min(chars.len());
        } else if chars[i] == '`' {
            match find(i + 1, &['`']) {
                Some(end) => {
                    let span: String = chars[i + 1..end].iter().collect();
                    let span = span.trim();
                    match span.strip_prefix('@').filter(|tag| is_name(tag).is_some()) {
                        Some(tag) => prose.push_str(&format!(" @{tag} ")),
                        None => prose.push(' '),
                    }
                    i = end + 1;
                }
                // A lone backtick is just a character
                None => {
                    prose.push(' ');
                    i += 1;
                }
            }
        } else {
            prose.push(chars[i]);
            i += 1;
        }
    }

    let mut out = HandoffMentions::default();
    for mention in parse_mentions_with_lookup(&prose, |_| Some(false)) {
        let tag = mention.name();
        if tag.eq_ignore_ascii_case("everyone") || tag.eq_ignore_ascii_case("all") {
            for i in 0..names.len() {
                if !out.targets.contains(&i) {
                    out.targets.push(i);
                }
            }
        } else if let Some(i) = is_name(tag) {
            if !out.targets.contains(&i) {
                out.targets.push(i);
            }
        } else if !NOT_A_BOT.iter().any(|w| w.eq_ignore_ascii_case(tag))
            && !out.unknown.iter().any(|u| u.eq_ignore_ascii_case(tag))
        {
            out.unknown.push(tag.to_string());
        }
    }
    for mention in parse_mentions_with_lookup(&fenced, |_| Some(false)) {
        if let Some(i) = is_name(mention.name()) {
            if !out.targets.contains(&i) && !out.in_code_block.contains(&i) {
                out.in_code_block.push(i);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_targets() {
        let names = ["Architect", "TechLead", "DevOps"];
        assert_eq!(handoff_targets("@techlead and @DevOps, say hi!", &names), vec![1, 2]);
        assert_eq!(handoff_targets("@DevOps then @devops again", &names), vec![2]);
        assert_eq!(handoff_targets("hey @everyone", &names), vec![0, 1, 2]);
        assert_eq!(handoff_targets("email me@TechLead.com or run `npm @DevOps`", &names), Vec::<usize>::new());
        assert_eq!(handoff_targets("run `@DevOps`", &names), vec![2]);
        assert_eq!(handoff_targets("@Stranger please help", &names), Vec::<usize>::new());
        assert_eq!(handoff_targets("```\n@Architect\n``` ok @Architect", &names), vec![0]);
    }

    /// The reply Product wrote in team-Startup on 2026-10-21: the model put the name in inline
    /// code, and Growth was never asked.
    #[test]
    fn a_name_in_inline_code_hands_on() {
        let names = ["Product", "TechLead", "Designer", "Security", "Growth"];
        let reply = "Security's and Designer's results cover what was asked, so no corrections were needed.\n\n\
            **One thing I need from you:** where should we host it?\n\n\
            `@Growth`, v0 is close to ready for the first 30 users. Please draft the launch plan: where to find \
            people who journal or set goals, the invite message, and a list of 5 people to interview.";
        assert_eq!(handoff_targets(reply, &names), vec![4]);
        assert_eq!(handoff_targets("ask ` @growth ` please", &names), vec![4], "spaces and case inside the span");
        assert_eq!(handoff_targets("`@Growth`/`@Security`", &names), vec![4, 3]);
        // Inline code that is more than a name stays code; so do fences and emails
        assert_eq!(handoff_targets("run `deploy @Growth` then", &names), Vec::<usize>::new());
        assert_eq!(handoff_targets("mail growth@Growth.io", &names), Vec::<usize>::new());
        assert_eq!(handoff_targets("an odd ` tick then @Growth", &names), vec![4], "a lone backtick hides nothing");
    }

    #[test]
    fn mentions_that_hand_nothing_on_are_reported() {
        let names = ["Product", "Growth", "Security"];
        let m = handoff_mentions("@Growth and @Marketer, cc @you\n```\n@Security run\n```\n@marketer again", &names);
        assert_eq!(m.targets, vec![1]);
        assert_eq!(m.unknown, vec!["Marketer".to_string()]);
        assert_eq!(m.in_code_block, vec![2]);
        // A teammate also named in prose isn't reported as only in code
        let m = handoff_mentions("```\n@Growth\n``` ok @Growth", &names);
        assert_eq!((m.targets, m.in_code_block), (vec![1], vec![]));
    }

    #[test]
    fn test_mention_methods() {
        let k = TagMention::Kwant("Architect".into());
        assert_eq!(k.name(), "Architect");
        assert!(k.is_kwant());
        assert!(!k.is_team());

        let t = TagMention::Team("core-engineering".into());
        assert_eq!(t.name(), "core-engineering");
        assert!(t.is_team());
        assert!(!t.is_kwant());
    }

    #[test]
    fn test_parse_mentions_edge_cases() {
        // Email addresses should not trigger mentions
        let mentions = parse_mentions("Send an email to user@example.com or @admin.");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].name(), "admin");
        assert!(mentions[0].is_kwant());

        // Trailing punctuation and parenthesis
        let mentions = parse_mentions("Ping (@lead-architect)! And @qa-team?");
        assert_eq!(mentions.len(), 2);
        assert!(mentions[0].is_team());
        assert_eq!(mentions[0].name(), "lead-architect");
        assert!(mentions[1].is_team());
        assert_eq!(mentions[1].name(), "qa-team");
    }

    #[test]
    fn test_parse_mentions_with_custom_lookup() {
        let input = "Hey @customteam and @special-kwant";
        let mentions = parse_mentions_with_lookup(input, |tag| match tag {
            "customteam" => Some(true),
            "special-kwant" => Some(false),
            _ => None,
        });
        assert_eq!(mentions.len(), 2);
        assert!(mentions[0].is_team());
        assert!(mentions[1].is_kwant());
    }
}

