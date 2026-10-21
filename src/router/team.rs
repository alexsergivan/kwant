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
        /// How the bot was picked: see [`Pick::basis`]. Empty from an older Laya that sends a decision itself.
        #[serde(default)]
        basis: String,
        #[serde(default)]
        matched_terms: Vec<String>,
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
        let laya = match self.query_laya_for_kwant(prompt, channel_name, members).await {
            Ok(data) => laya_guess(&data, members),
            Err(e) => {
                if self.status().is_up() {
                    tracing::warn!(error = %e, url = %self.base_url, "Laya channel routing failed, using keyword fallback");
                }
                None
            }
        };
        let fallback = laya.is_none();
        match pick_member(prompt, lead_kwant_id, members, laya, "channel lead") {
            Some(p) => ChannelRoute {
                kwant_id: p.kwant.id.clone(),
                reason: if fallback { format!("{}. {}", p.reason, self.offline_note()) } else { p.reason },
                fallback,
                matched_terms: p.terms,
                basis: p.basis.into(),
            },
            None => ChannelRoute {
                kwant_id: lead_kwant_id.to_string(),
                reason: "Routed to channel lead".into(),
                fallback,
                matched_terms: Vec::new(),
                basis: "lead".into(),
            },
        }
    }

    async fn query_laya_for_kwant(
        &self,
        prompt: &str,
        channel_name: &str,
        members: &[Kwant],
    ) -> Result<serde_json::Value, String> {
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
        if laya_guess(&data, members).is_none() {
            return Err("Laya response missing valid kwant choice".into());
        }
        Ok(data)
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
    let guess = laya_guess(data, members)?;
    Some((guess.kwant.id.clone(), format!("Laya matched this to {}'s role ({})", guess.kwant.name, guess.kwant.title)))
}

/// Laya's pick for `lead_kwant`, and whether it was clear of the runner-up. Laya only sees each bot's
/// name and title, so a near-tie between bots is a guess, not a match.
#[derive(Debug, Clone, Copy)]
pub struct LayaGuess<'a> {
    pub kwant: &'a Kwant,
    pub confident: bool,
}

fn laya_guess<'a>(data: &serde_json::Value, members: &'a [Kwant]) -> Option<LayaGuess<'a>> {
    let lead = data.get("answers")?.get("lead_kwant")?;
    let kwant = resolve_kwant_choice(lead.get("choice")?.as_str()?, members)?;
    let mut probs: Vec<f64> = lead
        .get("probabilities")
        .and_then(|p| p.as_object())
        .map(|p| p.values().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    probs.sort_by(|a, b| b.total_cmp(a));
    let confident = match probs.as_slice() {
        [top, second, ..] => top - second >= 0.15,
        _ => true,
    };
    Some(LayaGuess { kwant, confident })
}

/// Picks the member with the clearest match, or the lead when nothing (or several roles) match.
pub fn heuristic_pick_kwant<'a>(
    prompt: &str,
    lead_kwant_id: &str,
    members: &'a [Kwant],
) -> Option<(&'a Kwant, String)> {
    pick_member(prompt, lead_kwant_id, members, None, "channel lead").map(|p| (p.kwant, p.reason))
}

/// Who should answer, and the honest reason for the routing card.
#[derive(Debug, Clone)]
pub struct Pick<'a> {
    pub kwant: &'a Kwant,
    /// `only` (one bot can answer), `keywords` (words in the message point to this bot's role or
    /// name), `skills` (Laya and the words agree), `lead` (nothing matched), `several` (more than one
    /// role matched, so the lead coordinates), `team` (a request to put a team of bots together,
    /// which the lead proposes on one hire card). The UI words each one; see frontend/src/routeReason.ts.
    pub basis: &'static str,
    pub reason: String,
    pub terms: Vec<String>,
}

/// Engineering roles and the words in a request that belong to them. A bot has a role when its title
/// (or, without a title, the first line of its persona) names it; Laya only compares titles, so it
/// can't tell that "strategy" in a trading repo is backend code, and picked Frontend for it.
struct Role {
    titles: &'static [&'static str],
    /// Request words for this role, with weight: 2 is a clear signal, 1 a hint.
    signals: &'static [(&'static str, usize)],
}

const ROLES: &[Role] = &[
    Role {
        titles: &["frontend", "front-end", "ui", "ux", "interface", "designer", "web", "mobile", "ios", "android"],
        signals: &[
            ("ui", 2), ("ux", 2), ("frontend", 2), ("front-end", 2), ("page", 2), ("button", 2), ("css", 2),
            ("component", 2), ("layout", 2), ("screen", 2), ("react", 2), ("modal", 2), ("responsive", 2),
            ("style", 1), ("styling", 2), ("dashboard", 1), ("form", 1), ("color", 1), ("colour", 1), ("dark", 1),
        ],
    },
    Role {
        titles: &["backend", "back-end", "server", "api", "services", "database", "data", "go", "golang", "rust",
            "python", "java", "quant", "trading", "algorithm", "algorithms"],
        signals: &[
            ("backend", 2), ("back-end", 2), ("api", 2), ("endpoint", 2), ("database", 2), ("schema", 2),
            ("migration", 2), ("server", 2), ("service", 1), ("strategy", 2), ("strategies", 2), ("backtest", 2),
            ("trading", 2), ("indicator", 2), ("order", 1), ("orders", 1), ("algorithm", 2), ("query", 1),
            ("sql", 2), ("golang", 2), ("go", 1), ("rust", 1), ("function", 1), ("cli", 1), ("parser", 2),
        ],
    },
    Role {
        titles: &["qa", "quality", "test", "tests", "testing", "tester", "verification"],
        signals: &[
            ("tests", 2), ("testing", 2), ("qa", 2), ("e2e", 2), ("regression", 2), ("coverage", 2),
            ("flaky", 2), ("test", 1), ("verify", 1), ("bug", 1), ("repro", 2),
        ],
    },
    Role {
        titles: &["sre", "devops", "ops", "infrastructure", "infra", "platform", "reliability", "release"],
        signals: &[
            ("deploy", 2), ("deployment", 2), ("pipeline", 2), ("docker", 2), ("kubernetes", 2), ("k8s", 2),
            ("ci", 2), ("infra", 2), ("infrastructure", 2), ("monitoring", 2), ("alert", 2), ("alerts", 2),
            ("terraform", 2), ("outage", 2), ("uptime", 2), ("release", 1), ("logs", 1),
        ],
    },
];

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn roles_of(kwant: &Kwant) -> Vec<&'static Role> {
    let mut own = tokens(&kwant.title);
    own.extend(tokens(&kwant.name));
    let mut roles: Vec<&Role> = ROLES.iter().filter(|r| r.titles.iter().any(|t| own.iter().any(|o| o == t))).collect();
    if roles.is_empty() && kwant.title.trim().is_empty() {
        let first = tokens(kwant.persona_prompt.split(['.', '\n']).next().unwrap_or(""));
        roles = ROLES.iter().filter(|r| r.titles.iter().any(|t| first.iter().any(|o| o == t))).collect();
    }
    roles
}

/// The request words that belong to `kwant`'s role, and their weight.
fn role_score(words: &[String], kwant: &Kwant) -> (usize, Vec<String>) {
    let mut score = 0;
    let mut terms = Vec::new();
    for role in roles_of(kwant) {
        for (w, weight) in role.signals {
            if words.iter().any(|x| x == w) && !terms.iter().any(|t: &String| t == w) {
                score += weight;
                terms.push(w.to_string());
            }
        }
    }
    (score, terms)
}

fn lead_of<'a>(lead_kwant_id: &str, members: &'a [Kwant]) -> &'a Kwant {
    members.iter().find(|m| m.id == lead_kwant_id).unwrap_or(&members[0])
}

fn names(ks: &[&Kwant]) -> String {
    let n: Vec<String> = ks.iter().map(|k| format!("{} ({})", k.name, k.title)).collect();
    match n.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Routes one message to one member. In order: a bot named in the message; the lead for a request
/// to put a team of bots together; the one bot whose role the request's words belong to; the one
/// bot whose name, title or persona clearly shares the most words (not one everyday word, not a
/// tie); Laya's pick when it was clear. Anything else — nothing matched, or two roles matched —
/// goes to the lead, who answers or hands parts on. The reason says which of these happened.
pub fn pick_member<'a>(
    prompt: &str,
    lead_kwant_id: &str,
    members: &'a [Kwant],
    laya: Option<LayaGuess<'a>>,
    lead_word: &str,
) -> Option<Pick<'a>> {
    let only = match members {
        [] => return None,
        [one] => Some(one),
        _ => None,
    };
    if let Some(k) = only {
        return Some(Pick { kwant: k, basis: "only", reason: format!("{} is the only bot here that can answer", k.name), terms: vec![] });
    }
    let words = tokens(prompt);
    let lead = lead_of(lead_kwant_id, members);
    let to_lead = |matched: Vec<&'a Kwant>, terms: Vec<String>| {
        let reason = format!(
            "This touches several roles ({}), so it went to the {lead_word}, {} ({}), to answer or hand parts on",
            names(&matched),
            lead.name,
            lead.title
        );
        Pick { kwant: lead, basis: "several", reason, terms }
    };
    let chosen = |k: &'a Kwant, terms: Vec<String>, how: &str| {
        let agree = laya.is_some_and(|g| g.kwant.id == k.id);
        let mut reason = format!("{} ({}) {how}", k.name, k.title);
        if agree {
            reason.push_str("; Laya picked the same bot");
        } else if let Some(g) = laya {
            reason.push_str(&format!("; Laya guessed {} from titles alone, but nothing in the message points to that role", g.kwant.name));
        }
        Pick { kwant: k, basis: if agree { "skills" } else { "keywords" }, reason, terms }
    };

    // 1. Named in the message ("Bea, can you…")
    let named: Vec<&Kwant> = members.iter().filter(|m| words.iter().any(|w| *w == m.name.trim().to_lowercase())).collect();
    if let [k] = named.as_slice() {
        return Some(chosen(k, vec![k.name.trim().to_lowercase()], "was named in your message"));
    }

    // 2. A request to put a team of bots together is no one specialist's work: the lead proposes
    //    the bots on one hire card (see TEAM_BUILDING_NOTE)
    if is_team_building_request(prompt) {
        let reason = format!(
            "This asks to put a team of bots together, so it went to the {lead_word}, {} ({}), to propose them on one hire card for you to approve",
            lead.name, lead.title
        );
        return Some(Pick { kwant: lead, basis: "team", reason, terms: vec![] });
    }

    // 3. The request's words belong to a role
    let roles: Vec<(&Kwant, usize, Vec<String>)> =
        members.iter().map(|m| (m, role_score(&words, m))).map(|(m, (s, t))| (m, s, t)).collect();
    let strong: Vec<&(&Kwant, usize, Vec<String>)> = roles.iter().filter(|(_, s, _)| *s >= 2).collect();
    let distinct_roles = |set: &[&(&Kwant, usize, Vec<String>)]| {
        let mut terms: Vec<Vec<String>> = set.iter().map(|(_, _, t)| { let mut t = t.clone(); t.sort(); t }).collect();
        terms.dedup();
        terms.len()
    };
    if !strong.is_empty() {
        let top = strong.iter().map(|(_, s, _)| *s).max().unwrap_or(0);
        let best: Vec<&&(&Kwant, usize, Vec<String>)> = strong.iter().filter(|(_, s, _)| *s == top).collect();
        // Two bots with clear signals of different roles is multi-role work; two bots sharing one role
        // is a tie the lead settles.
        if distinct_roles(&strong) > 1 || best.len() > 1 {
            let matched: Vec<&Kwant> = strong.iter().map(|(m, _, _)| *m).collect();
            let mut terms: Vec<String> = Vec::new();
            for (_, _, t) in &strong {
                for w in t {
                    if !terms.contains(w) {
                        terms.push(w.clone());
                    }
                }
            }
            return Some(to_lead(matched, terms));
        }
        let (k, _, terms) = best[0];
        let how = format!("does this kind of work: your message mentions {}", terms.join(", "));
        return Some(chosen(k, terms.clone(), &how));
    }

    // 4. Shared words with a bot's name, title or persona (plus weak role hints). One shared word
    //    in a persona is chance, not a match, and a tie is no pick: both fall through to step 5
    //    rather than send the message to whoever happens to lead.
    let scored: Vec<(&Kwant, usize, Vec<String>)> = members
        .iter()
        .zip(&roles)
        .map(|(m, (_, rs, rt))| {
            let (s, mut t) = relevance(prompt, m);
            for w in rt {
                if !t.contains(w) {
                    t.push(w.clone());
                }
            }
            (m, s + rs, t)
        })
        .collect();
    let top = scored.iter().map(|(_, s, _)| *s).max().unwrap_or(0);
    let best: Vec<usize> = (0..scored.len()).filter(|&i| top > 0 && scored[i].1 == top).collect();
    if let [i] = best.as_slice() {
        // a hint of the bot's role ("orders" for a trading backend) counts on its own
        if top >= MIN_SHARED_SCORE || roles[*i].1 > 0 {
            let (k, _, terms) = &scored[*i];
            let how = format!("shares words with your message: {}", terms.join(", "));
            return Some(chosen(k, terms.clone(), &how));
        }
    }

    // 5. Nothing in the message matched: Laya's pick only when it was clear, else the lead
    if let Some(g) = laya.filter(|g| g.confident) {
        let reason = format!("Laya matched this to {}'s role ({}); no words in your message named a role", g.kwant.name, g.kwant.title);
        return Some(Pick { kwant: g.kwant, basis: "skills", reason, terms: vec![] });
    }
    let reason = format!("No one's role matched this, so it went to the {lead_word}, {} ({})", lead.name, lead.title);
    Some(Pick { kwant: lead, basis: "lead", reason, terms: vec![] })
}

/// The least a shared-word match must score to pick a bot: a word in its title (2), or more than
/// one word in its persona. A single persona word ("build" in a fitness coach's "build strength")
/// is chance.
const MIN_SHARED_SCORE: usize = 2;

/// Everyday words that say nothing about whose work a request is. They never count as shared
/// words with a bot's name, title or persona.
const GENERIC_WORDS: &[&str] = &[
    "team", "teams", "build", "building", "new", "need", "needed", "needs", "make", "making", "create",
    "creating", "do", "doing", "done", "everything", "anything", "something", "thing", "things", "help",
    "work", "working", "get", "start", "starting", "set", "setup", "analyze", "analyse", "analize", "bot",
    "bots", "agent", "agents", "good", "best", "first", "next", "now", "today", "also", "just", "like",
    "know", "look", "let", "lets", "let's", "go", "going", "think", "sure", "people", "way",
];

/// Said to the lead with a team-building request, so it proposes the bots instead of treating the
/// message as its own specialty.
pub const TEAM_BUILDING_NOTE: &str = "(kwant routing note: this message asks to put a team of bots together, \
    so it came to you as the lead. Work out which bots it needs, then propose them all on one card with \
    propose_hire, using members and new_team. The person approves the card; nothing is created before that.)";

/// "Create a team to…", "hire a few bots", "assemble a squad of agents": a request to put bots
/// together, as opposed to code that mentions teams ("build the team page", "add a team filter").
pub fn is_team_building_request(prompt: &str) -> bool {
    const VERBS: &[&str] = &[
        "create", "assemble", "build", "form", "hire", "recruit", "staff", "put", "spin", "set", "make", "start",
    ];
    const GROUPS: &[&str] = &["team", "teams", "crew", "squad", "bots", "agents", "specialists"];
    const MEMBERS: &[&str] = &["bot", "bots", "agent", "agents", "someone", "specialist", "specialists", "people"];
    /// After the group word, these make it a thing in the app rather than bots to hire.
    const OTHER_NOUNS: &[&str] = &[
        "page", "view", "screen", "component", "api", "endpoint", "table", "model", "settings", "list", "filter",
        "field", "route", "schema", "feature", "tab", "panel", "button", "modal", "form", "member", "members",
        "picker", "selector", "test", "tests", "id", "name", "lead", "channel", "description", "instructions",
        "workspace", "folder", "repo", "icon", "discussion", "meeting", "chat", "avatar", "card",
    ];
    /// What may stand between the verb and the group: "put together a small team", "spin up 3 agents".
    const FILLER: &[&str] = &[
        "a", "an", "the", "new", "small", "whole", "full", "together", "up", "my", "our", "some", "few", "couple",
        "of", "dedicated", "startup", "little", "proper", "entire", "me", "us", "virtual", "ai",
    ];
    let words = tokens(prompt);
    let is = |i: usize, set: &[&str]| words.get(i).is_some_and(|w| set.contains(&w.as_str()));
    words.iter().enumerate().any(|(i, w)| {
        // "hire a designer", "hire bots": hiring here is always about bots
        if w == "hire" || w == "recruit" {
            return (i + 1..i + 5).any(|j| is(j, MEMBERS) || is(j, GROUPS))
                || words.get(i + 1).is_some_and(|x| matches!(x.as_str(), "a" | "an" | "some" | "more" | "new"));
        }
        // "what bots we need", "which agents do we need"
        if matches!(w.as_str(), "what" | "which")
            && is(i + 1, &["bots", "agents", "specialists"])
            && (i + 2..i + 6).any(|j| words.get(j).is_some_and(|x| x == "need"))
        {
            return true;
        }
        // "create a team", "put together a small team of bots", "spin up 3 agents"
        if !VERBS.contains(&w.as_str()) {
            return false;
        }
        let mut j = i + 1;
        while j < words.len() && j <= i + 5 && (is(j, FILLER) || words[j].chars().all(|c| c.is_ascii_digit())) {
            j += 1;
        }
        is(j, GROUPS) && !is(j + 1, OTHER_NOUNS)
    })
}

/// "Create…", "Fix…", "Add…": a request to change code, which one role should do rather than discuss.
fn is_code_request(prompt: &str) -> bool {
    const VERBS: &[&str] = &[
        "create", "add", "implement", "write", "build", "make", "fix", "update", "change", "rename", "remove",
        "delete", "code", "port", "wire", "hook", "bump", "patch",
    ];
    let words = tokens(prompt);
    words.iter().take(3).any(|w| VERBS.contains(&w.as_str()))
}

/// A Direct decision from a pick. A message nobody can take goes to the lead.
fn direct(pick: Option<Pick<'_>>, team: &Team) -> TeamDecision {
    match pick {
        Some(p) => TeamDecision::Direct { kwant_id: p.kwant.id.clone(), reason: p.reason, basis: p.basis.into(), matched_terms: p.terms },
        None => TeamDecision::Direct {
            kwant_id: team.lead_kwant_id.clone(),
            reason: "No team members available; routed to team lead".into(),
            basis: "lead".into(),
            matched_terms: vec![],
        },
    }
}

/// A clear single-role code request, or a message addressed to one bot, goes straight to that bot even
/// when it reads like a discussion ("Create a new test simple strategy" used to start one).
fn settles_without_discussion(prompt: &str, pick: &Option<Pick<'_>>) -> bool {
    let Some(p) = pick else { return true };
    p.basis == "team" || p.reason.contains("was named in your message") || (is_code_request(prompt) && matches!(p.basis, "keywords" | "skills" | "only"))
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
    let guess = laya_guess(data, members);
    let pick = pick_member(prompt, &team.lead_kwant_id, members, guess, "team lead");
    let discuss = match discuss_probability {
        Some(p) => p >= 0.8 || (p >= 0.5 && is_complex_query(prompt)),
        None => choice == "discuss",
    } && !settles_without_discussion(prompt, &pick);

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
        Some(direct(pick, team))
    }
}

pub fn heuristic_team_decision(
    prompt: &str,
    team: &Team,
    members: &[Kwant],
) -> TeamDecision {
    let pick = pick_member(prompt, &team.lead_kwant_id, members, None, "team lead");
    if members.is_empty() || !is_complex_query(prompt) || settles_without_discussion(prompt, &pick) {
        return direct(pick, team);
    }

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
}

/// Words that ask for a discussion. Whole words only ("explain" isn't "plan"); "strategy" and
/// "design" are left out because in a code repo they are usually the thing to build.
fn is_complex_query(prompt: &str) -> bool {
    const COMPLEX: &[&str] = &[
        "architecture", "architectural", "tradeoff", "tradeoffs", "trade-off", "trade-offs", "migration",
        "migrations", "migrate", "plan", "review", "refactor", "overhaul", "rfc", "compare", "comparison", "vs",
        "versus",
    ];
    tokens(prompt).iter().any(|w| COMPLEX.contains(&w.as_str()))
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
    /// Prompt words found in the picked agent's name, title, persona or role.
    pub matched_terms: Vec<String>,
    /// How the pick was made: see [`Pick::basis`].
    pub basis: String,
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
        .filter(|s| s.len() >= 3 && !STOP_WORDS.contains(s) && !GENERIC_WORDS.contains(s))
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

