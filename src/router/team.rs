use crate::db::models::{Kwant, Team};
use crate::router::LayaRouter;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TeamDecision {
    Direct {
        kwant_id: String,
        reason: String,
    },
    Discuss {
        turns: usize,
        speakers: Vec<String>, // ordered kwant IDs
        plan: Vec<String>,
    },
}

impl LayaRouter {
    /// Determines whether a team query requires a multi-turn discussion between specialists
    /// or can be directly answered by a single kwant.
    ///
    /// If Laya service is reachable, queries /v1/systemone for decision orchestration.
    /// If Laya is unreachable or fails, falls back to intelligent keyword & expertise heuristics.
    pub async fn route_team_query(
        &self,
        prompt: &str,
        team: &Team,
        members: &[Kwant],
    ) -> TeamDecision {
        match self.query_laya_for_team(prompt, team, members).await {
            Ok(decision) => decision,
            Err(e) => {
                tracing::debug!(error = %e, "Laya team routing unavailable or failed, using heuristic fallback");
                heuristic_team_decision(prompt, team, members)
            }
        }
    }

    async fn query_laya_for_team(
        &self,
        prompt: &str,
        team: &Team,
        members: &[Kwant],
    ) -> Result<TeamDecision, String> {
        let kwant_criteria = kwant_choice_criteria(members);

        let payload = json!({
            "state": {
                "prompt": prompt,
                "team": {
                    "id": team.id,
                    "name": team.name,
                    "description": team.description.as_deref().unwrap_or("")
                }
            },
            "questions": {
                "team_decision": {
                    "type": "choice",
                    "instructions": "Determine whether this team query requires a collaborative discussion or a direct answer from one specialist.",
                    "criteria": {
                        "discuss": "Complex architectural decisions, trade-offs, multi-domain designs, migrations, reviews, or plans",
                        "direct": "Simple questions, status lookups, single-domain code implementations, quick queries"
                    }
                },
                "lead_kwant": {
                    "type": "choice",
                    "instructions": "Which kwant is best suited to answer directly or lead the discussion?",
                    "criteria": kwant_criteria
                }
            }
        });

        let endpoint = format!("{}/v1/systemone", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Laya HTTP status {}", resp.status()));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        // Support direct TeamDecision serialization if Laya directly outputs it
        if let Ok(decision) = serde_json::from_value::<TeamDecision>(data.clone()) {
            return Ok(decision);
        }

        // Support standard Laya answers structure
        if let Some(decision) = parse_laya_team_answers(prompt, &data, team, members) {
            return Ok(decision);
        }

        Err("Laya response missing valid team orchestration fields".into())
    }

    /// Picks the single best-suited kwant among channel members to answer a prompt.
    /// Returns `(kwant_id, reason)`. Falls back to keyword heuristics when Laya is unreachable.
    pub async fn route_channel_query(
        &self,
        prompt: &str,
        channel_name: &str,
        lead_kwant_id: &str,
        members: &[Kwant],
    ) -> (String, String) {
        let route = self.route_channel_query_detailed(prompt, channel_name, lead_kwant_id, members).await;
        (route.kwant_id, route.reason)
    }

    /// [`Self::route_channel_query`] plus how the pick was made, for the UI's "why this agent".
    pub async fn route_channel_query_detailed(
        &self,
        prompt: &str,
        channel_name: &str,
        lead_kwant_id: &str,
        members: &[Kwant],
    ) -> ChannelRoute {
        match self.query_laya_for_kwant(prompt, channel_name, members).await {
            Ok((kwant_id, reason)) => ChannelRoute { kwant_id, reason, fallback: false, matched_terms: Vec::new() },
            Err(e) => {
                if self.status().is_up() {
                    tracing::warn!(error = %e, url = %self.base_url, "Laya channel routing failed, using keyword fallback");
                }
                let (kwant_id, reason, matched_terms) = heuristic_pick_kwant(prompt, lead_kwant_id, members)
                    .map(|(k, r)| (k.id.clone(), r, matched_terms(prompt, k)))
                    .unwrap_or_else(|| (lead_kwant_id.to_string(), "Routed to channel lead".into(), Vec::new()));
                ChannelRoute {
                    kwant_id,
                    reason: format!("{reason}. {}", self.offline_note()),
                    fallback: true,
                    matched_terms,
                }
            }
        }
    }

    async fn query_laya_for_kwant(
        &self,
        prompt: &str,
        channel_name: &str,
        members: &[Kwant],
    ) -> Result<(String, String), String> {
        let kwant_criteria = kwant_choice_criteria(members);

        let payload = json!({
            "state": {
                "prompt": prompt,
                "channel": channel_name
            },
            "questions": {
                "lead_kwant": {
                    "type": "choice",
                    "instructions": "Which kwant is best suited to answer this question?",
                    "criteria": kwant_criteria
                }
            }
        });

        let endpoint = format!("{}/v1/systemone", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Laya HTTP status {}", resp.status()));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        parse_laya_kwant_choice(&data, members)
            .ok_or_else(|| "Laya response missing valid kwant choice".into())
    }
}

/// Laya scores each option by how well its key and description match the prompt. Opaque IDs and
/// long persona prompts drown that signal (it picked the first agent for almost everything), so
/// options are keyed by agent name and described by title alone, which measured far more accurate.
fn choice_keys(members: &[Kwant]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::with_capacity(members.len());
    for m in members {
        let name = m.name.trim();
        let mut key = name.to_string();
        let mut n = 2;
        while keys.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
            key = format!("{name} #{n}");
            n += 1;
        }
        keys.push(key);
    }
    keys
}

pub fn kwant_choice_criteria(members: &[Kwant]) -> serde_json::Map<String, serde_json::Value> {
    choice_keys(members)
        .into_iter()
        .zip(members)
        .map(|(key, m)| {
            let title = m.title.trim();
            let desc = if title.is_empty() {
                m.persona_prompt.split(['.', '\n']).next().unwrap_or("").trim().to_string()
            } else {
                title.to_string()
            };
            (key, serde_json::Value::String(desc))
        })
        .collect()
}

/// Maps Laya's choice (an option key, or an id/name if Laya echoes those) back to a member.
fn resolve_kwant_choice<'a>(choice: &str, members: &'a [Kwant]) -> Option<&'a Kwant> {
    let choice = choice.trim();
    choice_keys(members)
        .iter()
        .position(|k| k.eq_ignore_ascii_case(choice))
        .map(|i| &members[i])
        .or_else(|| members.iter().find(|m| m.id == choice || m.name.eq_ignore_ascii_case(choice)))
}

/// Extracts a valid member kwant choice from a Laya `answers.lead_kwant` response.
pub fn parse_laya_kwant_choice(data: &serde_json::Value, members: &[Kwant]) -> Option<(String, String)> {
    let choice = data
        .get("answers")?
        .get("lead_kwant")?
        .get("choice")?
        .as_str()?;
    let kwant = resolve_kwant_choice(choice, members)?;
    Some((
        kwant.id.clone(),
        format!("Laya picked {} ({}) as the best fit for this question", kwant.name, kwant.title),
    ))
}

/// Picks the member with the highest domain-keyword relevance, or the lead when nothing matches.
pub fn heuristic_pick_kwant<'a>(
    prompt: &str,
    lead_kwant_id: &str,
    members: &'a [Kwant],
) -> Option<(&'a Kwant, String)> {
    let (best, score) = members
        .iter()
        .map(|m| (m, calculate_kwant_relevance(prompt, m)))
        .fold(None, |acc: Option<(&Kwant, usize)>, (m, s)| match acc {
            Some((_, best_s)) if best_s >= s => acc,
            _ => Some((m, s)),
        })?;

    if score > 0 {
        Some((
            best,
            format!("Selected {} ({}) based on domain expertise", best.name, best.title),
        ))
    } else {
        let lead = members.iter().find(|m| m.id == lead_kwant_id).unwrap_or(&members[0]);
        Some((
            lead,
            format!(
                "No agent here lists skills matching this request, so {} ({}) is taking it as channel lead",
                lead.name, lead.title
            ),
        ))
    }
}

pub fn parse_laya_team_answers(
    prompt: &str,
    data: &serde_json::Value,
    team: &Team,
    members: &[Kwant],
) -> Option<TeamDecision> {
    let answers = data.get("answers")?;
    let choice = answers
        .get("team_decision")
        .and_then(|td| td.get("choice"))
        .and_then(|c| c.as_str())?;

    let lead = answers.get("lead_kwant");
    let lead_kwant = lead
        .and_then(|lk| lk.get("choice"))
        .and_then(|c| c.as_str())
        .and_then(|c| resolve_kwant_choice(c, members));
    let first_speaker = lead_kwant.map(|k| k.id.clone()).unwrap_or_else(|| team.lead_kwant_id.clone());

    // Laya leans "discuss" for nearly everything (greetings scored ~0.7, real topics 0.75-0.85), so it
    // only wins outright when very sure; otherwise the prompt itself must look like a discussion.
    let discuss_probability = answers
        .get("team_decision")
        .and_then(|td| td.get("probabilities"))
        .and_then(|p| p.get("discuss"))
        .and_then(|p| p.as_f64());
    let discuss = match discuss_probability {
        Some(p) => p >= 0.8 || (p >= 0.5 && is_complex_query(prompt)),
        None => choice == "discuss",
    };

    if discuss {
        // Laya's runner-up for the lead role is the best second voice; fall back to any other member.
        let mut ranked: Vec<(&Kwant, f64)> = lead
            .and_then(|lk| lk.get("probabilities"))
            .and_then(|p| p.as_object())
            .map(|probs| {
                probs
                    .iter()
                    .filter_map(|(k, v)| Some((resolve_kwant_choice(k, members)?, v.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let second_speaker = ranked
            .iter()
            .map(|(m, _)| *m)
            .chain(members.iter())
            .find(|m| m.id != first_speaker)
            .map(|m| m.id.clone())
            .unwrap_or_else(|| first_speaker.clone());

        let speakers = vec![first_speaker, second_speaker];
        let turns = speakers.len();
        let plan = generate_discussion_plan(&speakers, members);
        Some(TeamDecision::Discuss {
            turns,
            speakers,
            plan,
        })
    } else {
        let reason = match lead_kwant {
            Some(k) => format!("Laya picked {} ({}) as the best fit for this question", k.name, k.title),
            None => "Routed to the team lead".into(),
        };
        Some(TeamDecision::Direct {
            kwant_id: first_speaker,
            reason,
        })
    }
}

pub fn heuristic_team_decision(
    prompt: &str,
    team: &Team,
    members: &[Kwant],
) -> TeamDecision {
    if members.is_empty() {
        return TeamDecision::Direct {
            kwant_id: team.lead_kwant_id.clone(),
            reason: "No team members available; routed to team lead".into(),
        };
    }

    if is_complex_query(prompt) {
        // Multi-turn discussion between 2-3 specialists
        let lead_id = if members.iter().any(|m| m.id == team.lead_kwant_id) {
            team.lead_kwant_id.clone()
        } else {
            members[0].id.clone()
        };

        let mut other_members: Vec<&Kwant> = members.iter().filter(|m| m.id != lead_id).collect();
        // Rank other members by relevance to prompt
        other_members.sort_by_key(|m| std::cmp::Reverse(calculate_kwant_relevance(prompt, m)));

        let mut speakers = vec![lead_id];
        if !other_members.is_empty() {
            speakers.push(other_members[0].id.clone());
        }

        // If 3+ members and next member has specific relevance, include them (up to 3 turns)
        if other_members.len() >= 2 && calculate_kwant_relevance(prompt, other_members[1]) > 0 {
            speakers.push(other_members[1].id.clone());
        } else if speakers.len() < 2 {
            // If team only had 1 member
            speakers.push(speakers[0].clone());
        }

        let turns = speakers.len();
        let plan = generate_discussion_plan(&speakers, members);

        TeamDecision::Discuss {
            turns,
            speakers,
            plan,
        }
    } else {
        // Simple query or lookup: Direct to best suited Kwant
        let mut scored_members: Vec<(&Kwant, usize)> = members
            .iter()
            .map(|m| (m, calculate_kwant_relevance(prompt, m)))
            .collect();

        scored_members.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

        let best = if scored_members[0].1 > 0 {
            scored_members[0].0
        } else {
            // Fallback to team lead if no domain keyword matched
            members
                .iter()
                .find(|m| m.id == team.lead_kwant_id)
                .unwrap_or(&members[0])
        };

        TeamDecision::Direct {
            kwant_id: best.id.clone(),
            reason: format!(
                "Selected {} ({}) for direct response based on domain expertise",
                best.name, best.title
            ),
        }
    }
}

fn is_complex_query(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let complex_phrases = [
        "architecture",
        "architectural",
        "tradeoff",
        "trade-off",
        "design",
        "migration",
        "plan",
        "review",
        "refactor",
        "overhaul",
        "strategy",
        "rfc",
        "compare",
        "comparison",
    ];

    for phrase in complex_phrases {
        if lower.contains(phrase) {
            return true;
        }
    }

    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .collect();

    for w in words {
        if w == "vs" || w == "versus" {
            return true;
        }
    }

    false
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "you", "your", "are", "with", "this", "that", "what", "how", "can", "please",
    "from", "into", "about", "have", "has", "was", "were", "will", "would", "should", "could", "our",
    "who", "why", "when", "where", "which", "they", "them", "then", "than", "there", "here", "all",
    "any", "not", "but", "out", "use", "using", "does", "did", "its", "it's", "some", "need", "want",
];

/// A channel routing pick and how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRoute {
    pub kwant_id: String,
    pub reason: String,
    /// True when Laya was unreachable and keyword matching picked instead.
    pub fallback: bool,
    /// Prompt words found in the picked agent's name, title or persona (keyword fallback only).
    pub matched_terms: Vec<String>,
}

fn calculate_kwant_relevance(prompt: &str, kwant: &Kwant) -> usize {
    relevance(prompt, kwant).0
}

/// The prompt words that matched `kwant`, in prompt order, without repeats.
pub fn matched_terms(prompt: &str, kwant: &Kwant) -> Vec<String> {
    relevance(prompt, kwant).1
}

fn relevance(prompt: &str, kwant: &Kwant) -> (usize, Vec<String>) {
    let lower_prompt = prompt.to_lowercase();
    let words: Vec<&str> = lower_prompt
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| s.len() >= 3 && !STOP_WORDS.contains(s))
        .map(|s| if s.len() > 4 { s.strip_suffix('s').unwrap_or(s) } else { s })
        .collect();

    let name_lower = kwant.name.to_lowercase();
    let title_lower = kwant.title.to_lowercase();
    let prompt_lower = kwant.persona_prompt.to_lowercase();

    // Short words must match a whole token; longer ones may prefix-match ("container" ~ "containerization")
    let matches = |text: &str, w: &str| {
        text.split(|c: char| !c.is_alphanumeric() && c != '-')
            .any(|tok| tok == w || (w.len() >= 4 && tok.starts_with(w)))
    };

    let mut score = 0;
    let mut terms: Vec<String> = Vec::new();
    for w in words {
        let before = score;
        if matches(&name_lower, w) {
            score += 3;
        }
        if matches(&title_lower, w) {
            score += 2;
        }
        if matches(&prompt_lower, w) {
            score += 1;
        }
        if score > before && !terms.iter().any(|t| t == w) {
            terms.push(w.to_string());
        }
    }
    (score, terms)
}

fn generate_discussion_plan(speakers: &[String], members: &[Kwant]) -> Vec<String> {
    let total = speakers.len();
    speakers
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let name = members
                .iter()
                .find(|m| &m.id == id)
                .map(|m| m.name.as_str())
                .unwrap_or("Specialist");
            if i == 0 {
                format!("{}: Initial proposal and assessment", name)
            } else if i == total - 1 {
                format!("{}: Review, critique, and recommendation", name)
            } else {
                format!("{}: Domain analysis and feedback", name)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_fixture() -> (Team, Vec<Kwant>) {
        let team = Team {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "team-1".into(),
            name: "core-engineering".into(),
            description: Some("Architecture & DevOps".into()),
            workspace_path: None,
            repo_url: None,
            icon: "users".into(),
            lead_kwant_id: "kwant-1".into(),
            member_kwants: "[\"kwant-1\", \"kwant-2\"]".into(),
            created_at: None,
            instructions: None,
        };
        let members = vec![
            Kwant {
                space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
                id: "kwant-1".into(),
                name: "Architect".into(),
                title: "Systems Architect".into(),
                persona_prompt: "Architecture".into(),
                soul: String::new(),
                avatar_sprite: "architect".into(),
                driver_type: "api".into(),
                preferred_model: "auto".into(),
                temperature: 0.7,
                max_tokens: 4096,
                level: 1,
                turns_completed: 0,
                created_at: None,
            },
            Kwant {
                space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
                id: "kwant-2".into(),
                name: "DevOps".into(),
                title: "DevOps Engineer".into(),
                persona_prompt: "DevOps and deployments".into(),
                soul: String::new(),
                avatar_sprite: "devops".into(),
                driver_type: "api".into(),
                preferred_model: "auto".into(),
                temperature: 0.7,
                max_tokens: 4096,
                level: 1,
                turns_completed: 0,
                created_at: None,
            },
        ];
        (team, members)
    }

    #[test]
    fn test_heuristic_direct_decision() {
        let (team, members) = make_test_fixture();
        let decision = heuristic_team_decision("Check deployment status and pipeline logs", &team, &members);
        match decision {
            TeamDecision::Direct { kwant_id, .. } => {
                assert_eq!(kwant_id, "kwant-2");
            }
            _ => panic!("Expected Direct decision"),
        }
    }

    #[test]
    fn test_heuristic_lead_fallback_on_empty_keywords() {
        let (team, members) = make_test_fixture();
        let decision = heuristic_team_decision("hello there", &team, &members);
        match decision {
            TeamDecision::Direct { kwant_id, .. } => {
                assert_eq!(kwant_id, "kwant-1");
            }
            _ => panic!("Expected Direct decision to lead"),
        }
    }

    #[test]
    fn test_heuristic_pick_kwant_by_expertise_and_lead_fallback() {
        let (_, members) = make_test_fixture();
        let (k, _) = heuristic_pick_kwant("check deployment pipeline", "kwant-1", &members).unwrap();
        assert_eq!(k.id, "kwant-2");
        let (k, reason) = heuristic_pick_kwant("open google.com", "kwant-1", &members).unwrap();
        assert_eq!(k.id, "kwant-1");
        assert!(reason.contains("channel lead"));
        assert!(heuristic_pick_kwant("hi", "kwant-1", &[]).is_none());
    }

    #[test]
    fn test_parse_laya_kwant_choice() {
        let (_, members) = make_test_fixture();
        let ok = json!({ "answers": { "lead_kwant": { "choice": "kwant-2" } } });
        assert_eq!(parse_laya_kwant_choice(&ok, &members).unwrap().0, "kwant-2");
        let unknown = json!({ "answers": { "lead_kwant": { "choice": "ghost" } } });
        assert!(parse_laya_kwant_choice(&unknown, &members).is_none());
    }

    #[test]
    fn test_parse_laya_team_answers_json() {
        let (team, members) = make_test_fixture();
        let mock_discuss = json!({
            "answers": {
                "team_decision": { "choice": "discuss" },
                "lead_kwant": { "choice": "kwant-2" }
            }
        });
        let decision = parse_laya_team_answers("Plan the cluster migration", &mock_discuss, &team, &members).unwrap();
        match decision {
            TeamDecision::Discuss { speakers, turns, .. } => {
                assert_eq!(turns, 2);
                assert_eq!(speakers[0], "kwant-2");
                assert_eq!(speakers[1], "kwant-1");
            }
            _ => panic!("Expected Discuss decision"),
        }
    }
}

