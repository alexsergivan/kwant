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
        self.route_channel_query_after(prompt, channel_name, lead_kwant_id, members, None).await
    }

    /// [`Self::route_channel_query_detailed`] knowing which bot spoke last in the channel: a greeting
    /// or a message nothing matches goes to that bot (else the lead), not to a guess.
    pub async fn route_channel_query_after(
        &self,
        prompt: &str,
        channel_name: &str,
        lead_kwant_id: &str,
        members: &[Kwant],
        last_speaker: Option<&str>,
    ) -> ChannelRoute {
        self.route_channel_query_hinted(prompt, channel_name, lead_kwant_id, members, last_speaker, &Hints::default()).await
    }

    /// [`Self::route_channel_query_after`] with what the space knows beyond names and titles: each
    /// bot's covers/avoids lists (see [`Hints`]).
    pub async fn route_channel_query_hinted(
        &self,
        prompt: &str,
        channel_name: &str,
        lead_kwant_id: &str,
        members: &[Kwant],
        last_speaker: Option<&str>,
        hints: &Hints,
    ) -> ChannelRoute {
        let started = std::time::Instant::now();
        let mut why_not_laya = None;
        let laya = match self.query_laya_for_kwant(prompt, channel_name, members).await {
            Ok(data) => {
                let guess = laya_guess(&data, members);
                if guess.is_none() {
                    why_not_laya = Some("no usable pick");
                }
                guess
            }
            Err((kind, e)) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                if self.status().is_up() {
                    tracing::warn!(error = %e, reason = kind, url = %self.base_url, elapsed_ms, "Laya channel routing failed, using keyword fallback");
                } else {
                    tracing::info!(error = %e, reason = kind, elapsed_ms, "Laya didn't pick the bot for this message; using keyword fallback");
                }
                why_not_laya = Some(kind);
                None
            }
        };
        let fallback = laya.is_none();
        let route = match pick_member_hinted(prompt, lead_kwant_id, members, laya, "channel lead", last_speaker, hints) {
            Some(p) => ChannelRoute {
                kwant_id: p.kwant.id.clone(),
                reason: if fallback { format!("{}. {}", p.reason, self.offline_note()) } else { p.reason },
                fallback,
                matched_terms: p.terms,
                basis: p.basis.into(),
                fit: p.fit,
                choices: p.choices.iter().map(|k| k.id.clone()).collect(),
            },
            None => ChannelRoute {
                kwant_id: lead_kwant_id.to_string(),
                reason: "Routed to channel lead".into(),
                fallback,
                matched_terms: Vec::new(),
                basis: "lead".into(),
                fit: 0.0,
                choices: Vec::new(),
            },
        };
        // Every pick Laya didn't make is logged with why, so the fallback rate can be read from the log
        let reason = why_not_laya.or_else(|| fallback_reason(laya, &route)).or_else(|| {
            (laya.is_some_and(|g| g.kwant.id != route.kwant_id) && is_small_talk(prompt)).then_some("small talk")
        });
        if let Some(reason) = reason {
            tracing::info!(
                reason,
                basis = %route.basis,
                kwant_id = %route.kwant_id,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "channel message routed without Laya's pick"
            );
        }
        route
    }

    async fn query_laya_for_kwant(
        &self,
        prompt: &str,
        channel_name: &str,
        members: &[Kwant],
    ) -> Result<serde_json::Value, (&'static str, String)> {
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
            .map_err(|e| (error_kind(&e), e.to_string()))?;

        if !resp.status().is_success() {
            return Err(("error", format!("Laya HTTP status {}", resp.status())));
        }

        let data: serde_json::Value = resp.json().await.map_err(|e| (error_kind(&e), e.to_string()))?;
        if laya_guess(&data, members).is_none() {
            return Err(("error", "Laya response missing valid kwant choice".into()));
        }
        Ok(data)
    }
}

/// "timeout" when Laya took longer than [`super::laya::routing_timeout`], else "error".
fn error_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else {
        "error"
    }
}

/// Why a channel pick isn't Laya's when Laya did answer: "low confidence" when its guess was too
/// close to call and the pick went elsewhere. `None` when Laya's pick was used or agreed.
pub fn fallback_reason(laya: Option<LayaGuess<'_>>, route: &ChannelRoute) -> Option<&'static str> {
    let g = laya?;
    (!g.confident && g.kwant.id != route.kwant_id).then_some("low confidence")
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

pub fn laya_guess<'a>(data: &serde_json::Value, members: &'a [Kwant]) -> Option<LayaGuess<'a>> {
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
    /// How well the message fits this pick, 0 to 1: word evidence (role signals, words in the
    /// bot's name, title or persona), with Laya's agreement only as a nudge. A pick with no word
    /// evidence is 0, whatever Laya guessed. See [`ROUTE_AT`] and [`ASK_FROM`].
    pub fit: f32,
    /// The best candidates when the pick is close: the picked bot first, then the runner-up.
    pub choices: Vec<&'a Kwant>,
}

/// A channel message goes straight to the picked bot only at or above this fit.
pub const ROUTE_AT: f32 = 0.6;
/// From this fit up to [`ROUTE_AT`] the person is asked to choose between the best two bots (or a
/// general answer). Below it nothing in the message points at anyone.
pub const ASK_FROM: f32 = 0.4;

/// Roles and the words in a request that belong to them. A bot has a role when its title
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
            ("screens", 2), ("pages", 1), ("ipad", 1), ("iphone", 1),
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
            ("python", 1), ("script", 2), ("scripts", 2), ("scrape", 2), ("scraper", 2), ("crash", 2),
            ("crashes", 2), ("crashing", 2), ("traceback", 2), ("exception", 2), ("error", 1),
        ],
    },
    Role {
        titles: &["qa", "q-a", "quality", "test", "tests", "testing", "tester", "verification", "sdet"],
        signals: &[
            ("tests", 2), ("testing", 2), ("qa", 2), ("e2e", 2), ("regression", 2), ("coverage", 2),
            ("flaky", 2), ("test", 1), ("verify", 1), ("bug", 1), ("repro", 2),
            // Phrases only QA writes. Weight 1 so they don't outvote another role's clear words ("a test
            // plan for our ad campaign" is marketing's), but see NAMES_THE_WORK.
            ("test-plan", 1), ("test-plans", 1), ("test-case", 1), ("test-cases", 1), ("acceptance-test", 1),
            ("acceptance-tests", 1),
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
    Role {
        titles: &["designer", "design", "graphic", "visual", "brand", "branding", "illustrator", "art", "creative", "ux", "ui"],
        signals: &[
            ("logo", 2), ("logos", 2), ("palette", 2), ("mockup", 2), ("mockups", 2), ("wireframe", 2),
            ("wireframes", 2), ("figma", 2), ("usability", 2), ("typography", 2), ("illustration", 2), ("icon", 2),
            ("icons", 2), ("visuals", 2), ("branding", 2), ("redesign", 2), ("brand", 1), ("font", 1),
            ("prettier", 1), ("design", 1), ("visual", 1), ("color", 1), ("colour", 1),
        ],
    },
    Role {
        titles: &["marketing", "marketer", "growth", "seo", "copywriter", "content", "social", "community", "pr"],
        signals: &[
            ("marketing", 2), ("seo", 2), ("landing-page", 2), ("copywriting", 2), ("campaign", 2),
            ("campaigns", 2), ("ad", 2), ("ads", 2), ("tweet", 2), ("tweets", 2), ("newsletter", 2), ("blog", 2),
            ("tagline", 2), ("slogan", 2), ("headline", 2), ("headlines", 2), ("announcement", 2),
            ("instagram", 2), ("tiktok", 2), ("linkedin", 2), ("copy", 1), ("launch", 1), ("social", 1),
            ("audience", 1), ("conversion", 1), ("post", 1), ("seo-strategy", 2), ("keyword-strategy", 2),
            ("content-strategy", 2), ("marketing-strategy", 2), ("growth-strategy", 2), ("brand-strategy", 2),
            ("social-strategy", 2), ("launch-strategy", 2), ("a-b-test", 1), ("a-b-tests", 1), ("a-b-testing", 1),
        ],
    },
    Role {
        titles: &["research", "researcher", "analyst", "analytics", "insights"],
        signals: &[
            ("research", 2), ("market-size", 2), ("market-research", 2), ("survey", 2), ("surveys", 2),
            ("churn", 2), ("retention", 2), ("trends", 2), ("insights", 2), ("study", 2), ("studies", 2),
            ("market", 1), ("competitor", 1), ("competitors", 1), ("competition", 1), ("pricing", 1),
            ("compare", 1), ("analyze", 1), ("analyse", 1), ("analysis", 1), ("data", 1), ("report", 1),
            ("interviews", 1),
        ],
    },
];

/// Titles that say "engineer" without saying which kind. They count as the backend (code) role, but
/// only for a bot no other title word places: a "QA Engineer" is QA, a "Full-stack Software
/// Engineer" writes the code.
const GENERIC_ENGINEER_TITLES: &[&str] = &[
    "engineer", "engineering", "developer", "dev", "software", "full-stack", "fullstack", "programmer", "coder", "swe",
];

/// Two words that mean something else together: a landing page is marketing, not a UI page, an SEO
/// strategy isn't a trading strategy, and an A/B test is a marketing experiment. A pair can extend an
/// earlier one ("a-b" then "test").
const COMPOUNDS: &[(&str, &str)] = &[
    ("landing", "page"), ("market", "size"), ("market", "research"), ("test", "plan"), ("test", "plans"),
    ("test", "case"), ("test", "cases"), ("acceptance", "test"), ("acceptance", "tests"), ("seo", "strategy"),
    ("keyword", "strategy"), ("content", "strategy"), ("marketing", "strategy"), ("growth", "strategy"),
    ("brand", "strategy"), ("social", "strategy"), ("launch", "strategy"), ("front", "end"), ("back", "end"),
    ("q", "a"), ("a", "b"), ("a-b", "test"), ("a-b", "tests"), ("a-b", "testing"),
];

/// A matched role phrase such as "test plan" names the work as plainly as a title does, so a confident
/// Laya pick of another bot doesn't override it (a QA bot's own "test plan", not a guess from its persona).
fn names_the_work(term: &str) -> bool {
    term.contains('-') && COMPOUNDS.iter().any(|(a, b)| term == format!("{a}-{b}"))
}

/// [`tokens`] with [`COMPOUNDS`] joined by a hyphen, for role matching.
fn role_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in tokens(text) {
        if let Some(prev) = out.last_mut() {
            if COMPOUNDS.iter().any(|(a, b)| prev == a && w == *b) {
                *prev = format!("{prev}-{w}");
                continue;
            }
        }
        out.push(w);
    }
    out
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The roles that `text`'s words (a title, a name or a persona's first line) name.
fn roles_named_by(text: &str) -> Vec<&'static Role> {
    let own = role_words(text);
    let roles: Vec<&Role> = ROLES.iter().filter(|r| r.titles.iter().any(|t| own.iter().any(|o| o == t))).collect();
    if roles.is_empty() && own.iter().any(|o| GENERIC_ENGINEER_TITLES.contains(&o.as_str())) {
        return ROLES.iter().filter(|r| r.titles.contains(&"backend")).collect();
    }
    roles
}

/// A bot's roles come from its title. Its name only counts when the title names no role (a bot named
/// "Art" titled "Software Engineer" writes code, it isn't a designer), and the persona's first line
/// only when there is no title and the name names no role.
fn roles_of(kwant: &Kwant) -> Vec<&'static Role> {
    let from_title = roles_named_by(&kwant.title);
    if !from_title.is_empty() {
        return from_title;
    }
    let from_name = roles_named_by(&kwant.name);
    if !from_name.is_empty() || !kwant.title.trim().is_empty() {
        return from_name;
    }
    roles_named_by(kwant.persona_prompt.split(['.', '\n']).next().unwrap_or(""))
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
    // A role word in the bot's own name or title counts for it too: "tiktok" is on marketing's
    // list, but the "TikTok Scriptwriter" has it in its title
    for w in words {
        if !terms.contains(w) && in_own_title(kwant, w) && ROLES.iter().any(|r| r.signals.iter().any(|(s, _)| s == w)) {
            score += 2;
            terms.push(w.clone());
        }
    }
    (score, terms)
}

fn in_own_title(kwant: &Kwant, w: &str) -> bool {
    tokens(&format!("{} {}", kwant.name, kwant.title)).iter().any(|t| t == w)
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
    pick_member_after(prompt, lead_kwant_id, members, laya, lead_word, None)
}

/// [`pick_member`] knowing which bot spoke last. When nothing in the message points anywhere — a
/// greeting, "ok, go ahead", or no match while Laya wasn't sure or wasn't asked — the bot that spoke
/// last carries on, else the lead. Laya's titles-only guess sent "Hi" to a different bot each time.
pub fn pick_member_after<'a>(
    prompt: &str,
    lead_kwant_id: &str,
    members: &'a [Kwant],
    laya: Option<LayaGuess<'a>>,
    lead_word: &str,
    last_speaker: Option<&str>,
) -> Option<Pick<'a>> {
    pick_member_hinted(prompt, lead_kwant_id, members, laya, lead_word, last_speaker, &Hints::default())
}

/// What a space knows about its bots beyond names and titles.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Each bot's covers/avoids lists, by bot id. A bot without one routes as before.
    pub scopes: std::collections::HashMap<String, crate::db::routing::Scope>,
    /// Messages the person said should go to a bot (a correction or a choice on a card), newest first.
    pub learned: Vec<Learned>,
}

/// A message routing learned: its words and the bot it belongs to.
#[derive(Debug, Clone)]
pub struct Learned {
    pub kwant_id: String,
    pub words: Vec<String>,
}

impl Learned {
    pub fn new(kwant_id: &str, text: &str) -> Self {
        Learned { kwant_id: kwant_id.to_string(), words: content_words(text) }
    }
}

/// How much of the shorter message's words the longer one has, from 0 to 1, and how many words they
/// share; [`LEARNED_AT`] and two shared words make two messages alike.
pub fn word_overlap(a: &[String], b: &[String]) -> (f32, usize) {
    let shared = a.iter().filter(|w| b.contains(w)).count();
    let shorter = a.len().min(b.len());
    if shorter == 0 {
        return (0.0, 0);
    }
    (shared as f32 / shorter as f32, shared)
}

/// Two messages are alike: they share at least two words and [`LEARNED_AT`] of the shorter one's.
pub fn alike(a: &[String], b: &[String]) -> bool {
    let (overlap, shared) = word_overlap(a, b);
    shared >= 2 && overlap >= LEARNED_AT
}

/// Share of words a message must have in common with a learned one to route like it.
pub const LEARNED_AT: f32 = 0.6;

/// A message's words for comparing it with another: no filler, plurals folded, each once.
pub fn content_words(text: &str) -> Vec<String> {
    let mut w = scope_words(text);
    w.sort();
    w.dedup();
    w
}

impl Hints {
    /// The entries of `kwant`'s covers list that the message has every word of.
    fn covered(&self, kwant: &Kwant, words: &[String]) -> Vec<String> {
        self.scopes.get(&kwant.id).map(|s| s.covers.iter().filter(|e| entry_in(e, words)).map(|e| e.to_lowercase()).collect()).unwrap_or_default()
    }

    /// The member the most alike learned message belongs to, with how alike (see [`alike`]).
    fn learned_pick<'a>(&self, prompt: &str, members: &'a [Kwant]) -> Option<(&'a Kwant, f32)> {
        if self.learned.is_empty() {
            return None;
        }
        let words = content_words(prompt);
        let scoped = scope_words(prompt);
        let mut best: Option<(&'a Kwant, f32, usize)> = None;
        for l in &self.learned {
            let Some(k) = members.iter().find(|k| k.id == l.kwant_id) else { continue };
            if !alike(&words, &l.words) || self.avoids(k, &scoped) {
                continue;
            }
            let (overlap, shared) = word_overlap(&words, &l.words);
            // Newest first, so an equal match keeps the latest correction
            if best.is_none_or(|(_, o, s)| (overlap, shared) > (o, s)) {
                best = Some((k, overlap, shared));
            }
        }
        best.map(|(k, o, _)| (k, o))
    }

    /// The message has every word of an entry in `kwant`'s avoids list.
    fn avoids(&self, kwant: &Kwant, words: &[String]) -> bool {
        self.scopes.get(&kwant.id).is_some_and(|s| s.avoids.iter().any(|e| entry_in(e, words)))
    }
}

/// Words compared for covers/avoids: lowercased, a plural's "s" dropped ("restaurants" ~ "restaurant").
fn scope_words(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| if w.len() > 4 { w.strip_suffix('s').map(str::to_string).unwrap_or(w) } else { w })
        .collect()
}

/// Every word of a covers/avoids entry ("street food") is in the message.
fn entry_in(entry: &str, words: &[String]) -> bool {
    let need = scope_words(entry);
    !need.is_empty() && need.iter().all(|n| words.contains(n))
}

/// [`pick_member_after`] with [`Hints`]: an entry of a bot's covers list in the message counts
/// like a word of its title, and an entry of its avoids list keeps role and word matching off it
/// (naming or @mentioning it still picks it).
#[allow(clippy::too_many_arguments)]
pub fn pick_member_hinted<'a>(
    prompt: &str,
    lead_kwant_id: &str,
    members: &'a [Kwant],
    laya: Option<LayaGuess<'a>>,
    lead_word: &str,
    last_speaker: Option<&str>,
    hints: &Hints,
) -> Option<Pick<'a>> {
    let only = match members {
        [] => return None,
        [one] => Some(one),
        _ => None,
    };
    if let Some(k) = only {
        return Some(Pick { kwant: k, basis: "only", reason: format!("{} is the only bot here that can answer", k.name), terms: vec![], fit: 1.0, choices: vec![] });
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
        Pick { kwant: lead, basis: "several", reason, terms, fit: 0.7, choices: vec![] }
    };
    let laya_nudge = |k: &Kwant, base: f32| if base > 0.0 && laya.is_some_and(|g| g.kwant.id == k.id) { 0.1 } else { 0.0 };
    let chosen = |k: &'a Kwant, terms: Vec<String>, how: &str, base: f32| {
        let agree = laya.is_some_and(|g| g.kwant.id == k.id);
        let mut reason = format!("{} ({}) {how}", k.name, k.title);
        if agree {
            reason.push_str("; Laya picked the same bot");
        } else if let Some(g) = laya {
            reason.push_str(&format!("; Laya guessed {} from titles alone, but nothing in the message points to that role", g.kwant.name));
        }
        let fit = (base + laya_nudge(k, base)).min(1.0);
        Pick { kwant: k, basis: if agree { "skills" } else { "keywords" }, reason, terms, fit, choices: vec![] }
    };

    // 1. @Named in the message: one @Name wins over plain names ("@Frontend why didn't you answer
    //    QA" is for Frontend, not "Frontend and QA"); two or more @Names go to the lead
    let member_names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let at_named: Vec<&'a Kwant> =
        crate::router::handoff_mentions(prompt, &member_names).targets.into_iter().map(|i| &members[i]).collect();
    match at_named.as_slice() {
        [] => {}
        [k] => return Some(chosen(k, vec![k.name.trim().to_lowercase()], "was @mentioned in your message", 1.0)),
        several => {
            let terms = several.iter().map(|k| k.name.trim().to_lowercase()).collect();
            let reason = format!(
                "Your message @mentions {}, so it went to the {lead_word}, {} ({}), to answer or hand parts on",
                names(several),
                lead.name,
                lead.title
            );
            return Some(Pick { kwant: lead, basis: "several", reason, terms, fit: 1.0, choices: vec![] });
        }
    }
    //    Named without @ ("Bea, can you…")
    let named: Vec<&Kwant> = members.iter().filter(|m| words.iter().any(|w| *w == m.name.trim().to_lowercase())).collect();
    if let [k] = named.as_slice() {
        return Some(chosen(k, vec![k.name.trim().to_lowercase()], "was named in your message", 1.0));
    }

    // 2. A request to put a team of bots together is no one specialist's work: the lead proposes
    //    the bots on one hire card (see TEAM_BUILDING_NOTE)
    if is_team_building_request(prompt) {
        let reason = format!(
            "This asks to put a team of bots together, so it went to the {lead_word}, {} ({}), to propose them on one hire card for you to approve",
            lead.name, lead.title
        );
        return Some(Pick { kwant: lead, basis: "team", reason, terms: vec![], fit: 1.0, choices: vec![] });
    }

    // 2b. The person said a message like this one should go to a bot (a correction or a choice on
    //     a card): that bot, unless its avoids list keeps it out
    if let Some((k, overlap)) = hints.learned_pick(prompt, members) {
        let reason = format!("{} ({}) answers messages like this one: you chose it for a similar message", k.name, k.title);
        let fit = (ROUTE_AT + (1.0 - ROUTE_AT) * overlap).min(1.0);
        return Some(Pick { kwant: k, basis: "learned", reason, terms: vec![], fit, choices: vec![] });
    }

    // 3. The request's words belong to a role
    let role_words = role_words(prompt);
    let scope_words = scope_words(prompt);
    let avoided = |k: &Kwant| hints.avoids(k, &scope_words);
    let roles: Vec<(&Kwant, usize, Vec<String>)> = members
        .iter()
        .map(|m| (m, if avoided(m) { (0, vec![]) } else { role_score(&role_words, m) }))
        .map(|(m, (s, t))| (m, s, t))
        .collect();
    let strong: Vec<&(&Kwant, usize, Vec<String>)> = roles.iter().filter(|(_, s, _)| *s >= 2).collect();
    let distinct_roles = |set: &[&(&Kwant, usize, Vec<String>)]| {
        let mut terms: Vec<Vec<String>> = set.iter().map(|(_, _, t)| { let mut t = t.clone(); t.sort(); t }).collect();
        terms.dedup();
        terms.len()
    };
    // A word that reaches one bot through its own title and others only through their role's list
    // is that bot's
    let owners: Vec<&&(&Kwant, usize, Vec<String>)> = strong.iter().filter(|(m, _, t)| t.iter().any(|w| in_own_title(m, w))).collect();
    if let [owner] = owners.as_slice() {
        let (k, s, terms) = **owner;
        if strong.iter().all(|(_, _, t)| t.iter().all(|w| terms.contains(w))) {
            let how = format!("does this kind of work: your message mentions {}", terms.join(", "));
            return Some(chosen(k, terms.clone(), &how, (0.5 + 0.1 * *s as f32).min(1.0)));
        }
    }
    if !strong.is_empty() {
        let top = strong.iter().map(|(_, s, _)| *s).max().unwrap_or(0);
        let best: Vec<&&(&Kwant, usize, Vec<String>)> = strong.iter().filter(|(_, s, _)| *s == top).collect();
        // Two bots with clear signals of different roles is multi-role work; two bots sharing one role
        // is a tie the lead settles.
        if distinct_roles(&strong) > 1 || best.len() > 1 {
            let matched: Vec<&Kwant> = strong.iter().map(|(m, _, _)| *m).collect();
            let same_role_tie = distinct_roles(&strong) == 1;
            let mut terms: Vec<String> = Vec::new();
            for (_, _, t) in &strong {
                for w in t {
                    if !terms.contains(w) {
                        terms.push(w.clone());
                    }
                }
            }
            let mut p = to_lead(matched.clone(), terms);
            if same_role_tie {
                // Two bots with the same role: a close call between them, not multi-role work
                p.fit = 0.5;
                p.choices = matched.into_iter().take(2).collect();
            }
            return Some(p);
        }
        let (k, s, terms) = best[0];
        let how = format!("does this kind of work: your message mentions {}", terms.join(", "));
        return Some(chosen(k, terms.clone(), &how, (0.5 + 0.1 * *s as f32).min(1.0)));
    }

    // 4. Shared words with a bot's name, title or persona (plus weak role hints). A word several
    //    bots share says nothing about which of them it is for ("plan" for a fitness coach's
    //    "workout plans" and a travel planner), so it counts for none of them; one word only one
    //    bot has ("workout", "flight") is enough. A tie is no pick: both fall through to step 5
    //    rather than send the message to whoever happens to lead.
    //    An entry of a bot's covers list counts like a word of its title.
    let matches: Vec<Vec<(String, usize)>> = members
        .iter()
        .map(|m| {
            if avoided(m) {
                return vec![];
            }
            let mut terms = relevance_terms(prompt, m);
            for e in hints.covered(m, &scope_words) {
                if !terms.iter().any(|(t, _)| *t == e) {
                    terms.push((e, 2));
                }
            }
            terms
        })
        .collect();
    let shared = |w: &str| matches.iter().filter(|terms| terms.iter().any(|(t, _)| t == w)).count() > 1;
    // (bot, score, terms, whether a term matched its name or title rather than only its persona)
    let scored: Vec<(&Kwant, usize, Vec<String>, bool)> = members
        .iter()
        .zip(&roles)
        .zip(&matches)
        .map(|((m, (_, rs, rt)), terms)| {
            let own: Vec<&(String, usize)> = terms.iter().filter(|(t, _)| !shared(t)).collect();
            let s = own.iter().map(|(_, s)| s).sum::<usize>();
            let named = own.iter().any(|(_, s)| *s >= 2) || rt.iter().any(|t| names_the_work(t));
            let mut t: Vec<String> = own.iter().map(|(t, _)| t.clone()).collect();
            for w in rt {
                if !t.contains(w) {
                    t.push(w.clone());
                }
            }
            (m, s + rs, t, named)
        })
        .collect();
    let top = scored.iter().map(|(_, s, _, _)| *s).max().unwrap_or(0);
    let best: Vec<usize> = (0..scored.len()).filter(|&i| top > 0 && scored[i].1 == top).collect();
    // Words only in a bot's persona, or a weak role hint ("plan" in "writes test plans", "page"),
    // are a guess too, so a clear Laya pick of another bot wins over them.
    let laya_clear = laya.filter(|g| g.confident);
    let weak = |i: usize| !scored[i].3;
    if let ([i], Some(g)) = (best.as_slice(), laya_clear) {
        if weak(*i) && scored[*i].0.id != g.kwant.id {
            let reason = format!(
                "Laya matched this to {}'s role ({}); the only words in your message that pointed elsewhere were in {}'s description",
                g.kwant.name, g.kwant.title, scored[*i].0.name
            );
            // Laya's titles-only guess against words only in a description: neither is evidence
            // enough on its own, so it's a close call between the two
            return Some(Pick { kwant: g.kwant, basis: "skills", reason, terms: vec![], fit: 0.5, choices: vec![g.kwant, scored[*i].0] });
        }
    }
    if let [i] = best.as_slice() {
        let (k, s, terms, named) = &scored[*i];
        let how = format!("shares words with your message: {}", terms.join(", "));
        // One word only in a description is a hint (0.5); a word in the name or title, or two
        // description words, is enough to route
        let base = (0.4 + 0.1 * *s as f32 + if *named { 0.1 } else { 0.0 }).min(0.95);
        let mut p = chosen(k, terms.clone(), &how, base);
        let runner_up = (0..scored.len())
            .filter(|&j| j != *i && scored[j].1 > 0)
            .max_by_key(|&j| scored[j].1)
            .map(|j| scored[j].0);
        p.choices = std::iter::once(*k).chain(runner_up).collect();
        return Some(p);
    }
    let tied: Vec<&'a Kwant> = best.iter().map(|&i| scored[i].0).take(2).collect();

    // 5. Nothing in the message matched (or two bots tied on words): the bot that spoke last, else
    //    the lead, with no fit. Laya alone never picks: it sees only titles, and its confident guess
    //    sent "Best place to eat corn dogs in Munich tomorrow?" to a TikTok scriptwriter. A tie is a
    //    close call between the tied bots.
    let (fit, choices) = if tied.len() > 1 { (0.45, tied) } else { (0.0, vec![]) };
    if let Some(k) = last_speaker.and_then(|id| members.iter().find(|m| m.id == id)).filter(|k| !avoided(k)) {
        // "thanks", "make it shorter", "and for Berlin?" carry on with whoever is talking
        let fit = if choices.is_empty() && (is_small_talk(prompt) || reads_as_follow_up(prompt)) { ROUTE_AT } else { fit };
        let reason = format!("Nothing in your message named a role, so {} ({}), who spoke last here, carries on", k.name, k.title);
        return Some(Pick { kwant: k, basis: "last", reason, terms: vec![], fit, choices });
    }
    let reason = format!("No one's role matched this, so it went to the {lead_word}, {} ({})", lead.name, lead.title);
    Some(Pick { kwant: lead, basis: "lead", reason, terms: vec![], fit, choices })
}

/// Everyday words that say nothing about whose work a request is. They never count as shared
/// words with a bot's name, title or persona.
const GENERIC_WORDS: &[&str] = &[
    "team", "teams", "build", "building", "new", "need", "needed", "needs", "make", "making", "create",
    "creating", "do", "doing", "done", "everything", "anything", "something", "thing", "things", "help",
    "work", "working", "get", "start", "starting", "set", "setup", "analyze", "analyse", "analize", "bot",
    "bots", "agent", "agents", "good", "best", "first", "next", "now", "today", "also", "just", "like",
    "know", "look", "let", "lets", "let's", "go", "going", "think", "sure", "people", "way", "find",
    "finding", "send", "sends", "sending", "sent", "manage", "managing", "give", "show", "tell", "every",
    "please", "want", "thanks", "thank",
];

/// Words that greet, thank or agree: a message of only these (and everyday words) names no one's work.
const SMALL_TALK: &[&str] = &[
    "hi", "hello", "hey", "hiya", "yo", "morning", "evening", "afternoon", "night", "thanks", "thank", "thx", "ok",
    "okay", "yes", "yeah", "yep", "no", "nope", "sure", "great", "cool", "nice", "perfect", "ahead", "go", "do",
    "it", "please", "good", "fine", "agreed", "continue", "proceed", "there", "all", "you", "everyone",
];

/// A greeting, a thank-you or a go-ahead ("Hi", "ok, go ahead", "thanks!"): nothing in it for
/// Laya or keywords to match, so it stays with whoever is talking.
pub fn is_small_talk(prompt: &str) -> bool {
    let words = tokens(prompt);
    !words.is_empty()
        && words.len() <= 6
        && words.iter().all(|w| SMALL_TALK.contains(&w.as_str()) || STOP_WORDS.contains(&w.as_str()))
}

/// A short reply or one that leans on what was just said ("make it shorter", "and Berlin?", "what
/// about tomorrow"): it belongs to the bot that spoke last, even with nothing in it to match.
pub fn reads_as_follow_up(prompt: &str) -> bool {
    const STARTS: &[&str] = &[
        "and", "also", "then", "now", "so", "but", "ok", "okay", "it", "that", "this", "those", "these", "again",
        "another", "more", "same", "make", "shorter", "longer", "what about", "how about", "why", "can you make",
    ];
    let lower = prompt.trim().to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return false;
    }
    let joined = words.join(" ");
    words.len() <= 3 || STARTS.iter().any(|s| joined == *s || joined.starts_with(&format!("{s} ")))
}

/// What the channel does with a pick, given its fit and the room it was made in.
#[derive(Debug, Clone, PartialEq)]
pub enum Settle {
    /// Send it to the picked bot.
    Route,
    /// A close call: ask the person to choose between these two bots (ids, best first) or a
    /// general answer.
    Ask(Vec<String>),
    /// Nothing in the message points at anyone here: the Concierge answers.
    Concierge,
}

/// A fit of at least [`ROUTE_AT`] routes. A close call ([`ASK_FROM`] up to [`ROUTE_AT`], with two
/// candidates) asks the person. Below that, #general (every bot in the space) leans to the
/// Concierge rather than to whichever specialist happened to lead or speak last; any other channel
/// leans to its own members, so a low score stays with the last speaker or lead as before.
pub fn settle(route: &ChannelRoute, general: bool, has_concierge: bool) -> Settle {
    if route.fit >= ROUTE_AT {
        return Settle::Route;
    }
    if route.fit >= ASK_FROM && route.choices.len() >= 2 {
        return Settle::Ask(route.choices.iter().take(2).cloned().collect());
    }
    if general && has_concierge {
        return Settle::Concierge;
    }
    Settle::Route
}

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
    /// Verbs that also go with a team in everyday use, so they need a bot word too.
    const WEAK_VERBS: &[&str] = &["put", "make", "start"];
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
        if !is(j, GROUPS) || is(j + 1, OTHER_NOUNS) {
            return false;
        }
        // "put the team on the dashboard", "make the team happy": everyday verbs only put a team of
        // bots together when bots are said ("put together a team of agents", "make 3 bots")
        !WEAK_VERBS.contains(&w.as_str()) || is(j, MEMBERS) || (j + 1..j + 4).any(|k| is(k, MEMBERS))
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
/// when it reads like a discussion ("Create a new test simple strategy" used to start one). So does a
/// message whose only discussion word is an everyday one ("plan"), whose own words point to one
/// specialist, and that names no other bot: "need a new workout plan" is FitnessCoach's, and shouldn't
/// start a discussion that puts TravelPlanner first. "Plan the migration…" still is a discussion.
fn settles_without_discussion(prompt: &str, pick: &Option<Pick<'_>>, members: &[Kwant]) -> bool {
    if is_lookup_nudge(prompt) {
        return true;
    }
    let Some(p) = pick else { return true };
    if p.basis == "team" || p.reason.contains("was named in your message") || (is_code_request(prompt) && matches!(p.basis, "keywords" | "skills" | "only")) {
        return true;
    }
    let words = tokens(prompt);
    let named = members.iter().filter(|m| words.iter().any(|w| *w == m.name.trim().to_lowercase())).count();
    matches!(p.basis, "keywords" | "skills") && !p.terms.is_empty() && named <= 1 && !has_strong_discussion_word(prompt)
}

/// A follow-up telling the team to look its own questions up ("You can answer your questions by
/// checking the site!") goes to one bot to do the research; a new discussion would only ask again.
fn is_lookup_nudge(prompt: &str) -> bool {
    const LOOKUP: &[&str] = &["check", "checking", "look", "lookup", "research", "search", "google", "find", "read", "visit"];
    const BACK: &[&str] = &["question", "questions", "yourself", "yourselves", "site", "website", "page", "docs"];
    // Other asks in the same message ("design it, write the copy, and check the docs", "…and discuss") are real work
    const WORK: &[&str] = &["discuss", "propose", "design", "write", "create", "build", "draft", "launch", "decide", "summarize"];
    let words = tokens(prompt);
    !is_complex_query(prompt)
        && words.len() <= 12
        && !words.iter().any(|w| WORK.contains(&w.as_str()))
        && words.iter().any(|w| LOOKUP.contains(&w.as_str()))
        && words.iter().any(|w| BACK.contains(&w.as_str()))
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
    } && !settles_without_discussion(prompt, &pick, members);

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
    if members.is_empty() || !is_complex_query(prompt) || settles_without_discussion(prompt, &pick, members) {
        return direct(pick, team);
    }

    // Multi-turn discussion between 2-3 specialists
    let lead_id = if members.iter().any(|m| m.id == team.lead_kwant_id) {
        team.lead_kwant_id.clone()
    } else {
        members[0].id.clone()
    };

    // Other members whose name, title, persona or role shares words with the message, best first.
    // When none does, an engineering-role member may still join an engineering discussion, but a bot
    // with no such role has nothing to add: "review my pull request" used to bring in FitnessCoach
    // because it was listed first. With nobody to add, the lead answers alone.
    let words = tokens(prompt);
    let score = |m: &Kwant| calculate_kwant_relevance(prompt, m) + role_score(&words, m).0;
    let mut other_members: Vec<(&Kwant, usize)> =
        members.iter().filter(|m| m.id != lead_id).map(|m| (m, score(m))).filter(|(_, s)| *s > 0).collect();
    other_members.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    if other_members.is_empty() {
        other_members.extend(members.iter().filter(|m| m.id != lead_id && !roles_of(m).is_empty()).take(1).map(|m| (m, 0)));
    }
    if other_members.is_empty() && members.len() > 1 {
        return direct(pick, team);
    }

    let mut speakers = vec![lead_id];
    // Up to two other voices (3 turns)
    speakers.extend(other_members.iter().take(2).map(|(m, _)| m.id.clone()));
    if speakers.len() < 2 {
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
    tokens(prompt).iter().any(|w| w == "plan") || has_strong_discussion_word(prompt)
}

/// The discussion words other than "plan", which everyday requests use for one person's work
/// ("a workout plan", "plan a trip").
fn has_strong_discussion_word(prompt: &str) -> bool {
    const COMPLEX: &[&str] = &[
        "architecture", "architectural", "tradeoff", "tradeoffs", "trade-off", "trade-offs", "migration",
        "migrations", "migrate", "review", "refactor", "overhaul", "rfc", "compare", "comparison", "vs",
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
    /// See [`Pick::fit`].
    pub fit: f32,
    /// See [`Pick::choices`]: bot ids, best first.
    pub choices: Vec<String>,
}

fn calculate_kwant_relevance(prompt: &str, kwant: &Kwant) -> usize {
    relevance(prompt, kwant).0
}

/// The prompt words that matched `kwant`, in prompt order, without repeats.
pub fn matched_terms(prompt: &str, kwant: &Kwant) -> Vec<String> {
    relevance(prompt, kwant).1
}

fn relevance(prompt: &str, kwant: &Kwant) -> (usize, Vec<String>) {
    let terms = relevance_terms(prompt, kwant);
    (terms.iter().map(|(_, s)| s).sum(), terms.into_iter().map(|(t, _)| t).collect())
}

/// The prompt words that matched `kwant`'s name (3), title (2) or persona (1), each with its score,
/// in prompt order, without repeats.
fn relevance_terms(prompt: &str, kwant: &Kwant) -> Vec<(String, usize)> {
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

    let mut terms: Vec<(String, usize)> = Vec::new();
    for w in words {
        let mut score = 0;
        if matches(&name_lower, w) {
            score += 3;
        }
        if matches(&title_lower, w) {
            score += 2;
        }
        if matches(&prompt_lower, w) {
            score += 1;
        }
        if score > 0 && !terms.iter().any(|(t, _)| t == w) {
            terms.push((w.to_string(), score));
        }
    }
    terms
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
    fn a_nudge_to_look_it_up_goes_to_one_bot_not_a_new_discussion() {
        let (team, members) = make_test_fixture();
        let sure = json!({
            "answers": {
                "team_decision": { "choice": "discuss", "probabilities": { "discuss": 0.9 } },
                "lead_kwant": { "choice": "kwant-1" }
            }
        });
        for nudge in ["You can answer on your questions by checking site!", "look it up yourself", "read the docs page"] {
            let decision = parse_laya_team_answers(nudge, &sure, &team, &members).unwrap();
            assert!(matches!(decision, TeamDecision::Direct { .. }), "{nudge}");
            assert!(matches!(heuristic_team_decision(nudge, &team, &members), TeamDecision::Direct { .. }), "{nudge}");
        }
        // A real planning request still starts a discussion
        let plan = parse_laya_team_answers("Plan the site migration and check the docs", &sure, &team, &members).unwrap();
        assert!(matches!(plan, TeamDecision::Discuss { .. }));
    }

    #[test]
    fn a_lookup_word_inside_real_multi_part_work_still_discusses() {
        let (team, members) = make_test_fixture();
        let sure = json!({"answers": {"team_decision": {"choice": "discuss", "probabilities": {"discuss": 0.9}}, "lead_kwant": {"choice": "kwant-1"}}});
        for p in [
            "Should we use Postgres or SQLite? Check the docs and discuss",
            "Look at our site and propose a new onboarding flow",
            "Launch the new pricing page: design it, write the copy, and check the docs",
        ] {
            let d = parse_laya_team_answers(p, &sure, &team, &members).unwrap();
            assert!(matches!(d, TeamDecision::Discuss { .. }), "{p}");
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

    fn community() -> Kwant {
        let (_, m) = make_test_fixture();
        Kwant {
            id: "kwant-3".into(),
            name: "Community".into(),
            title: "Comments & Audience Manager".into(),
            persona_prompt: "Replies to comments and sends messages to the audience".into(),
            ..m[0].clone()
        }
    }

    /// F2a: one generic verb ("send") no longer wins the pick; nothing specific goes to the lead.
    #[test]
    fn a_generic_verb_alone_does_not_pick_a_bot() {
        let (_, mut members) = make_test_fixture();
        members.push(community());
        let p = pick_member_after("Every morning send me an overview of my day at 7:00", "kwant-1", &members, None, "lead", None).unwrap();
        assert_eq!(p.kwant.id, "kwant-1", "{p:?}");
        assert!(!p.terms.iter().any(|t| t == "send"), "{p:?}");
    }

    /// F2a: a greeting goes to whoever spoke last (else the lead), never to Laya's titles-only guess.
    #[test]
    fn a_greeting_stays_with_the_last_speaker_not_laya() {
        let (_, mut members) = make_test_fixture();
        members.push(community());
        let guess = Some(LayaGuess { kwant: &members[1], confident: true });
        let p = pick_member_after("Hi!", "kwant-1", &members, guess, "lead", Some("kwant-3")).unwrap();
        assert_eq!((p.kwant.id.as_str(), p.basis), ("kwant-3", "last"), "{p:?}");
        let p = pick_member_after("Hi!", "kwant-1", &members, guess, "lead", None).unwrap();
        assert_eq!(p.kwant.id, "kwant-1", "{p:?}");
        // A last speaker who isn't a member here doesn't count
        let p = pick_member_after("hello there", "kwant-1", &members, None, "lead", Some("stranger")).unwrap();
        assert_eq!(p.kwant.id, "kwant-1", "{p:?}");
        // Laya's clear pick alone no longer routes real work: nothing in the message named a role,
        // so it has no fit (the channel hands it to the Concierge in #general)
        let p = pick_member_after("Can you look into why the build broke?", "kwant-1", &members, guess, "lead", Some("kwant-3")).unwrap();
        assert_eq!((p.kwant.id.as_str(), p.basis, p.fit), ("kwant-3", "last", 0.0), "{p:?}");
        // A follow-up stays with the last speaker
        let p = pick_member_after("make it shorter", "kwant-1", &members, guess, "lead", Some("kwant-3")).unwrap();
        assert_eq!((p.kwant.id.as_str(), p.fit), ("kwant-3", ROUTE_AT), "{p:?}");
        assert!(is_small_talk("Hi") && is_small_talk("thanks, ok!") && !is_small_talk("hi, deploy the app"));
        assert!(!is_small_talk(""));
    }

    /// Oleks 2026-10-29: "@Frontend why you did not answer to QA" named two bots, so it went to
    /// the lead. One @Name wins over a plain name; two @Names still go to the lead.
    #[test]
    fn one_at_mention_wins_over_plain_names() {
        let (_, mut members) = make_test_fixture();
        let base = members[0].clone();
        let named = |id: &str, name: &str, title: &str| Kwant { id: id.into(), name: name.into(), title: title.into(), ..base.clone() };
        members.extend([
            named("kwant-fe", "Frontend", "Frontend and UX engineer"),
            named("kwant-qa", "QA", "QA and DevOps engineer"),
            named("kwant-de", "Designer", "Product and UI designer"),
        ]);
        let sure_qa = Some(LayaGuess { kwant: &members[members.len() - 2], confident: true });
        let p = pick_member_after("@Frontend why you did not answer to QA. It mentioned you", "kwant-1", &members, sure_qa, "lead", Some("kwant-qa"))
            .unwrap();
        assert_eq!((p.kwant.id.as_str(), p.basis), ("kwant-fe", "keywords"), "{p:?}");
        assert!(p.reason.contains("@mentioned"), "{p:?}");
        // Inline code that is just the name counts too, as in a chip
        let p = pick_member_after("`@Frontend` please answer QA's design report", "kwant-1", &members, None, "lead", None).unwrap();
        assert_eq!(p.kwant.id, "kwant-fe", "{p:?}");
        // Two @Names: the lead
        let p = pick_member_after("@Frontend and @QA, sort out the chip focus", "kwant-1", &members, None, "lead", None).unwrap();
        assert_eq!((p.kwant.id.as_str(), p.basis), ("kwant-1", "several"), "{p:?}");
        assert!(p.reason.contains("Frontend") && p.reason.contains("QA"), "{p:?}");
        // An email address or an unknown @name is no pick: the old rules apply
        let p = pick_member_after("mail ops@frontend.example about QA", "kwant-1", &members, None, "lead", None).unwrap();
        assert_ne!(p.kwant.id, "kwant-fe", "{p:?}");
        let p = pick_member_after("@Nobody, Frontend please look", "kwant-1", &members, None, "lead", None).unwrap();
        assert_eq!(p.kwant.id, "kwant-fe", "{p:?}");
    }

    /// F2a: a pick that went against Laya's unsure guess is logged as "low confidence".
    #[test]
    fn a_low_confidence_guess_is_named_as_the_fallback_reason() {
        let (_, members) = make_test_fixture();
        let route = |id: &str| ChannelRoute { kwant_id: id.into(), reason: String::new(), fallback: false, matched_terms: vec![], basis: "lead".into(), fit: 0.0, choices: vec![] };
        let unsure = Some(LayaGuess { kwant: &members[1], confident: false });
        assert_eq!(fallback_reason(unsure, &route("kwant-1")), Some("low confidence"));
        assert_eq!(fallback_reason(unsure, &route("kwant-2")), None);
        let sure = Some(LayaGuess { kwant: &members[1], confident: true });
        assert_eq!(fallback_reason(sure, &route("kwant-2")), None);
        assert_eq!(fallback_reason(None, &route("kwant-1")), None);
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    fn bot(id: &str, name: &str, title: &str, persona: &str) -> Kwant {
        Kwant {
            id: id.into(),
            name: name.into(),
            title: title.into(),
            persona_prompt: persona.into(),
            soul: String::new(),
            avatar_sprite: String::new(),
            driver_type: "claude".into(),
            preferred_model: String::new(),
            temperature: 0.7,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
        }
    }

    fn general() -> Vec<Kwant> {
        vec![
            bot("k-lead", "Luna", "Booktun.ing Chief", "You coordinate the booktun.ing marketing team."),
            bot("k-script", "Scriptwriter", "TikTok Scriptwriter & Hook Specialist", "You write short TikTok video scripts and scroll-stopping hooks for book content."),
            bot("k-seo", "Atlas", "Technical SEO", "You audit sites for crawl errors, sitemaps and page speed."),
        ]
    }

    fn route_of(p: &Pick<'_>) -> ChannelRoute {
        ChannelRoute {
            kwant_id: p.kwant.id.clone(),
            reason: p.reason.clone(),
            fallback: false,
            matched_terms: p.terms.clone(),
            basis: p.basis.into(),
            fit: p.fit,
            choices: p.choices.iter().map(|k| k.id.clone()).collect(),
        }
    }

    /// Oleks 2026-11-01: in #general "Best place to eat corn dogs in Munich tomorrow?" went to the
    /// TikTok scriptwriter on Laya's confident titles-only guess. No word in it fits anyone, so it
    /// has no fit and #general hands it to the Concierge.
    #[test]
    fn corn_dogs_in_munich_go_to_the_concierge() {
        let members = general();
        let laya = Some(LayaGuess { kwant: &members[1], confident: true });
        let prompt = "Best place to eat corn dogs in Munich tomorrow?";
        for last in [None, Some("k-script"), Some("k-seo")] {
            let p = pick_member_after(prompt, "k-lead", &members, laya, "channel lead", last).unwrap();
            assert_eq!(p.fit, 0.0, "{p:?}");
            assert_ne!(p.basis, "skills", "{p:?}");
            assert_eq!(settle(&route_of(&p), true, true), Settle::Concierge);
            // Without a Concierge (the person removed it), or outside #general, today's behaviour
            assert_eq!(settle(&route_of(&p), true, false), Settle::Route);
            assert_eq!(settle(&route_of(&p), false, true), Settle::Route);
        }
    }

    #[test]
    fn a_clear_specialist_request_still_routes_straight_to_it() {
        let members = general();
        for laya in [None, Some(LayaGuess { kwant: &members[1], confident: true })] {
            let p = pick_member_after("Write a TikTok hook for this fantasy book", "k-lead", &members, laya, "channel lead", Some("k-seo")).unwrap();
            assert_eq!(p.kwant.id, "k-script", "{p:?}");
            assert!(p.fit >= ROUTE_AT, "{p:?}");
            assert_eq!(settle(&route_of(&p), true, true), Settle::Route);
        }
        let p = pick_member_after("Why is the sitemap missing pages?", "k-lead", &members, None, "channel lead", None).unwrap();
        assert_eq!(p.kwant.id, "k-seo", "{p:?}");
        assert!(p.fit >= ROUTE_AT, "{p:?}");
    }

    #[test]
    fn laya_only_nudges_a_pick_with_word_evidence() {
        let members = general();
        // One description-only word is a hint below the bar; Laya agreeing tips it over
        let prompt = "any thoughts on scroll-stopping openers?";
        let alone = pick_member_after(prompt, "k-lead", &members, None, "channel lead", None).unwrap();
        let agreed = pick_member_after(prompt, "k-lead", &members, Some(LayaGuess { kwant: &members[1], confident: true }), "channel lead", None).unwrap();
        assert_eq!(alone.kwant.id, "k-script", "{alone:?}");
        assert!(alone.fit < ROUTE_AT && alone.fit >= ASK_FROM, "{alone:?}");
        assert!(agreed.fit > alone.fit, "{agreed:?}");
    }

    #[test]
    fn follow_ups_and_greetings_stay_with_the_last_speaker() {
        let members = general();
        for prompt in ["thanks!", "make it shorter", "and for Berlin?", "what about the second one"] {
            let p = pick_member_after(prompt, "k-lead", &members, None, "channel lead", Some("k-script")).unwrap();
            assert_eq!((p.kwant.id.as_str(), p.basis), ("k-script", "last"), "{prompt}: {p:?}");
            assert_eq!(settle(&route_of(&p), true, true), Settle::Route, "{prompt}");
        }
        assert!(!reads_as_follow_up("Best place to eat corn dogs in Munich tomorrow?"));
    }

    fn hints(id: &str, covers: &[&str], avoids: &[&str]) -> Hints {
        let scope = crate::db::routing::Scope {
            covers: covers.iter().map(|s| s.to_string()).collect(),
            avoids: avoids.iter().map(|s| s.to_string()).collect(),
        };
        Hints { scopes: [(id.to_string(), scope)].into_iter().collect(), ..Hints::default() }
    }

    /// Two bots with the same role and the same words: a close call, so the person is asked to
    /// choose between them (or a general answer), in #general and elsewhere. @mentions never ask.
    #[test]
    fn a_close_call_asks_the_person() {
        let members = vec![
            bot("k-fe", "Bea", "Frontend engineer", "You build the app's pages."),
            bot("k-web", "Cy", "Web developer", "You build websites."),
            bot("k-seo", "Atlas", "Technical SEO", "You audit sites."),
        ];
        let p = pick_member_after("The button CSS is off on mobile Safari", "k-seo", &members, None, "channel lead", None).unwrap();
        assert!((ASK_FROM..ROUTE_AT).contains(&p.fit), "{p:?}");
        let ask = Settle::Ask(vec!["k-fe".into(), "k-web".into()]);
        assert_eq!(settle(&route_of(&p), true, true), ask);
        assert_eq!(settle(&route_of(&p), false, false), ask);
        let p = pick_member_after("@Bea the button CSS is off", "k-seo", &members, None, "channel lead", None).unwrap();
        assert_eq!((p.kwant.id.as_str(), settle(&route_of(&p), true, true)), ("k-fe", Settle::Route));
    }

    /// An entry of a bot's covers list counts like a word of its title; empty lists change nothing.
    #[test]
    fn covers_route_a_message_to_the_bot_that_covers_it() {
        let members = general();
        let prompt = "Best place to eat corn dogs in Munich tomorrow?";
        let none = pick_member_hinted(prompt, "k-lead", &members, None, "channel lead", None, &Hints::default()).unwrap();
        assert_eq!(none.fit, 0.0);
        let h = hints("k-lead", &["corn dogs", "restaurants"], &[]);
        let p = pick_member_hinted(prompt, "k-lead", &members, None, "channel lead", None, &h).unwrap();
        assert_eq!(p.kwant.id, "k-lead", "{p:?}");
        assert!(p.fit >= ROUTE_AT, "{p:?}");
        // A plural matches its singular entry
        let p = pick_member_hinted("Any restaurants near the venue?", "k-seo", &members, None, "channel lead", None, &h).unwrap();
        assert_eq!(p.kwant.id, "k-lead", "{p:?}");
    }

    /// An entry of a bot's avoids list keeps role and word matching off it; naming it still works.
    #[test]
    fn avoids_keep_a_bot_out_unless_it_is_named() {
        let members = general();
        let h = hints("k-script", &[], &["fantasy"]);
        let prompt = "Write a TikTok hook for this fantasy book";
        let p = pick_member_hinted(prompt, "k-lead", &members, None, "channel lead", Some("k-script"), &h).unwrap();
        assert_ne!(p.kwant.id, "k-script", "{p:?}");
        let p = pick_member_hinted("Scriptwriter, a hook for this fantasy book", "k-lead", &members, None, "channel lead", None, &h).unwrap();
        assert_eq!(p.kwant.id, "k-script", "{p:?}");
    }

    /// The person chose the Scriptwriter for a message no title points to: a similar one routes
    /// there too (basis "learned"), an unlike one doesn't, and @naming another bot still wins.
    #[test]
    fn a_learned_example_routes_a_similar_message() {
        let members = general();
        let learned = Hints { learned: vec![Learned::new("k-script", "Can you draft the monthly book club newsletter?")], ..Hints::default() };
        let prompt = "Draft this month's book club newsletter please";
        let before = pick_member_hinted(prompt, "k-lead", &members, None, "channel lead", None, &Hints::default()).unwrap();
        assert_ne!(before.kwant.id, "k-script", "{before:?}");
        let p = pick_member_hinted(prompt, "k-lead", &members, None, "channel lead", None, &learned).unwrap();
        assert_eq!((p.kwant.id.as_str(), p.basis), ("k-script", "learned"), "{p:?}");
        assert!(p.fit >= ROUTE_AT, "{p:?}");
        let p = pick_member_hinted("Why do the book pages load slowly?", "k-lead", &members, None, "channel lead", None, &learned).unwrap();
        assert_ne!(p.basis, "learned", "{p:?}");
        let p = pick_member_hinted("@Atlas draft the book club newsletter", "k-lead", &members, None, "channel lead", None, &learned).unwrap();
        assert_eq!(p.kwant.id, "k-seo");
        // A bot that isn't in the channel isn't picked
        let without = [members[0].clone(), members[2].clone()];
        let p = pick_member_hinted(prompt, "k-lead", &without, None, "channel lead", None, &learned).unwrap();
        assert_ne!(p.basis, "learned");
    }
}
