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

#[cfg(test)]
mod tests {
    use super::*;

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

