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
        let mut kwant_criteria = serde_json::Map::new();
        for m in members {
            let desc = format!("{} ({}): {}", m.name, m.title, m.persona_prompt);
            kwant_criteria.insert(m.id.clone(), serde_json::Value::String(desc));
        }

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
        if let Some(decision) = parse_laya_team_answers(&data, team, members) {
            return Ok(decision);
        }

        Err("Laya response missing valid team orchestration fields".into())
    }
}

pub fn parse_laya_team_answers(
    data: &serde_json::Value,
    team: &Team,
    members: &[Kwant],
) -> Option<TeamDecision> {
    let answers = data.get("answers")?;
    let choice = answers
        .get("team_decision")
        .and_then(|td| td.get("choice"))
        .and_then(|c| c.as_str())?;

    let lead_choice = answers
        .get("lead_kwant")
        .and_then(|lk| lk.get("choice"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());

    if choice == "discuss" {
        let first_speaker = lead_choice.unwrap_or_else(|| team.lead_kwant_id.clone());
        let second_speaker = members
            .iter()
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
        let kwant_id = lead_choice.unwrap_or_else(|| team.lead_kwant_id.clone());
        Some(TeamDecision::Direct {
            kwant_id,
            reason: "Laya orchestrator selected specialist for direct response".into(),
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

fn calculate_kwant_relevance(prompt: &str, kwant: &Kwant) -> usize {
    let lower_prompt = prompt.to_lowercase();
    let words: Vec<&str> = lower_prompt
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| s.len() >= 3)
        .collect();

    let name_lower = kwant.name.to_lowercase();
    let title_lower = kwant.title.to_lowercase();
    let prompt_lower = kwant.persona_prompt.to_lowercase();

    let mut score = 0;
    for w in words {
        if name_lower.contains(w) {
            score += 3;
        }
        if title_lower.contains(w) {
            score += 2;
        }
        if prompt_lower.contains(w) {
            score += 1;
        }
    }
    score
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
            id: "team-1".into(),
            name: "core-engineering".into(),
            description: Some("Architecture & DevOps".into()),
            workspace_path: None,
            repo_url: None,
            icon: "users".into(),
            lead_kwant_id: "kwant-1".into(),
            member_kwants: "[\"kwant-1\", \"kwant-2\"]".into(),
            created_at: None,
        };
        let members = vec![
            Kwant {
                id: "kwant-1".into(),
                name: "Architect".into(),
                title: "Systems Architect".into(),
                persona_prompt: "Architecture".into(),
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
                id: "kwant-2".into(),
                name: "DevOps".into(),
                title: "DevOps Engineer".into(),
                persona_prompt: "DevOps and deployments".into(),
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
    fn test_parse_laya_team_answers_json() {
        let (team, members) = make_test_fixture();
        let mock_discuss = json!({
            "answers": {
                "team_decision": { "choice": "discuss" },
                "lead_kwant": { "choice": "kwant-2" }
            }
        });
        let decision = parse_laya_team_answers(&mock_discuss, &team, &members).unwrap();
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

