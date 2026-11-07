//! The three ready-made teams (spec T1/T2): Inbox, Sales and Coding. The data is Mira's generated
//! `design/templates.js`, converted byte for byte into `templates.json` by
//! `scripts/gen-team-data.mjs`; nothing here rewrites it.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub handle: String,
    pub card_line: String,
    pub mission: String,
    /// The team's pinned instructions. Written at create, not served in the catalogue.
    #[serde(skip_serializing)]
    pub instructions: String,
    pub needs: Vec<Need>,
    pub never: Vec<String>,
    pub routine: Option<Routine>,
    pub first_prompts: Vec<String>,
    pub bots: Vec<TemplateBot>,
    pub skills: Vec<TemplateSkill>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Need {
    pub id: String,
    pub label: String,
    pub why: String,
    pub required: bool,
    pub opens: String,
    pub done_when: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub name: String,
    pub when: String,
    #[serde(skip_serializing)]
    pub cron: String,
    /// The slot of the bot that runs it.
    pub bot: String,
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBot {
    pub preset: String,
    pub lead: bool,
    /// The template's own name for this bot; it is also the slot key in `names` (api.md §1).
    pub name: String,
    pub title: String,
    pub avatar_sprite: String,
    pub line: String,
    #[serde(skip_serializing)]
    pub persona: String,
    #[serde(skip_serializing)]
    pub soul: String,
    pub good_for: Vec<String>,
    pub not_for: Vec<String>,
}

impl TemplateBot {
    pub fn slot(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSkill {
    /// The slot of the bot that owns it.
    pub owner: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

pub fn all() -> &'static [Template] {
    static ALL: OnceLock<Vec<Template>> = OnceLock::new();
    ALL.get_or_init(|| serde_json::from_str(include_str!("templates.json")).expect("src/teams/templates.json is valid"))
}

pub fn get(id: &str) -> Option<&'static Template> {
    all().iter().find(|t| t.id == id)
}

/// One catalogue entry as served by `GET /api/team-templates` (api.md §1).
#[derive(Serialize)]
pub struct CatalogueEntry<'a> {
    #[serde(flatten)]
    pub template: CatalogueTemplate<'a>,
    pub used_in_space: bool,
}

#[derive(Serialize)]
pub struct CatalogueTemplate<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub icon: &'a str,
    pub handle: &'a str,
    pub card_line: &'a str,
    pub mission: &'a str,
    pub bots: Vec<CatalogueBot<'a>>,
    pub needs: &'a [Need],
    pub never: &'a [String],
    pub skills: &'a [TemplateSkill],
    pub routine: Option<&'a Routine>,
    pub first_prompts: &'a [String],
}

#[derive(Serialize)]
pub struct CatalogueBot<'a> {
    pub slot: &'a str,
    #[serde(flatten)]
    pub bot: &'a TemplateBot,
}

impl Template {
    pub fn catalogue(&self) -> CatalogueTemplate<'_> {
        CatalogueTemplate {
            id: &self.id,
            label: &self.label,
            icon: &self.icon,
            handle: &self.handle,
            card_line: &self.card_line,
            mission: &self.mission,
            bots: self.bots.iter().map(|b| CatalogueBot { slot: b.slot(), bot: b }).collect(),
            needs: &self.needs,
            never: &self.never,
            skills: &self.skills,
            routine: self.routine.as_ref(),
            first_prompts: &self.first_prompts,
        }
    }

    pub fn lead(&self) -> &TemplateBot {
        self.bots.iter().find(|b| b.lead).expect("every template has a lead")
    }

    pub fn bot(&self, slot: &str) -> Option<&TemplateBot> {
        self.bots.iter().find(|b| b.slot() == slot)
    }
}

/// Limits a skill must meet to be inserted (the same as `POST /api/skills`).
pub const MAX_SKILL_BODY_CHARS: usize = 8000;
pub const MAX_SKILL_NAME_CHARS: usize = 48;
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 200;
/// `good_for` / `not_for` caps (the `kwant_scope` editor's limits).
pub const MAX_SCOPE_ITEMS: usize = 20;
pub const MAX_SCOPE_CHARS: usize = 60;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::{names, presets};
    use std::collections::HashSet;

    #[test]
    fn three_templates_in_order() {
        let ids: Vec<_> = all().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["inbox", "sales", "coding"]);
        assert!(get("coding").unwrap().routine.is_none());
        assert!(get("nope").is_none());
    }

    #[test]
    fn every_bot_is_valid_and_has_a_known_preset() {
        let mut handles = HashSet::new();
        for t in all() {
            assert!(names::valid_handle(&t.handle), "{}", t.handle);
            assert!(handles.insert(&t.handle), "duplicate handle {}", t.handle);
            assert_eq!(t.bots.iter().filter(|b| b.lead).count(), 1, "{} needs one lead", t.id);
            assert!(t.bots.len() <= 6);
            let mut slots = HashSet::new();
            for b in &t.bots {
                assert!(presets::get(&b.preset).is_some(), "{}: unknown preset {}", t.id, b.preset);
                assert!(names::valid_name(&b.name), "{}: bad name {:?}", t.id, b.name);
                assert!(slots.insert(b.name.to_lowercase()), "{}: duplicate slot {}", t.id, b.name);
                assert!(!b.persona.trim().is_empty() && !b.soul.trim().is_empty());
                for list in [&b.good_for, &b.not_for] {
                    assert!(list.len() <= MAX_SCOPE_ITEMS);
                    assert!(list.iter().all(|s| !s.trim().is_empty() && s.chars().count() <= MAX_SCOPE_CHARS), "{}: {list:?}", b.name);
                }
            }
            assert_eq!(t.never.len(), 3, "{}", t.id);
        }
    }

    #[test]
    fn sales_requires_what_you_sell_and_it_is_the_only_composer_need() {
        // A5 (decisions.md): copy from design/templates-copy.md, Sales "Needs (T5)"
        let sales = get("sales").unwrap();
        let n = sales.needs.iter().find(|n| n.id == "what_you_sell").expect("Sales has what_you_sell");
        assert_eq!(n.label, "Tell the team what you sell");
        assert_eq!(n.why, "Three short answers in this chat. Leads are only as good as this.");
        assert!(n.required);
        assert_eq!(n.opens, "composer_prompt");
        assert_eq!(n.done_when, "The person has posted a message in @sales");
        let composer: Vec<_> = all().iter().flat_map(|t| t.needs.iter().filter(|n| n.opens == "composer_prompt").map(move |n| (t.id.as_str(), n.id.as_str()))).collect();
        assert_eq!(composer, [("sales", "what_you_sell")]);
    }

    #[test]
    fn skills_fit_and_have_owners() {
        for t in all() {
            for s in &t.skills {
                assert!(t.bot(&s.owner).is_some(), "{}: skill owner {}", t.id, s.owner);
                assert!(s.body.chars().count() <= MAX_SKILL_BODY_CHARS, "{}", s.name);
                assert!((1..=MAX_SKILL_NAME_CHARS).contains(&s.name.chars().count()), "{}", s.name);
                assert!(s.description.chars().count() <= MAX_SKILL_DESCRIPTION_CHARS, "{}", s.name);
            }
        }
    }

    #[test]
    fn routines_are_off_owned_and_valid_cron() {
        for t in all() {
            if let Some(r) = &t.routine {
                assert!(!r.enabled, "{}: routine must ship paused", t.id);
                assert!(t.bot(&r.bot).is_some(), "{}: routine bot {}", t.id, r.bot);
                assert!(!r.prompt.trim().is_empty());
                let s = crate::routines::Schedule::Cron { expression: r.cron.clone(), tz: "UTC".into() };
                s.validate().unwrap_or_else(|e| panic!("{}: {} {e}", t.id, r.cron));
            }
        }
    }

    #[test]
    fn catalogue_hides_persona_soul_and_instructions() {
        let v = serde_json::to_value(get("inbox").unwrap().catalogue()).unwrap();
        let text = v.to_string();
        assert!(v["bots"][0]["slot"] == "InboxManager");
        for hidden in ["\"persona\"", "\"soul\"", "\"instructions\"", "\"cron\"", "\"prompt\""] {
            assert!(!text.contains(hidden), "{hidden} leaked");
        }
        assert!(v["skills"][0]["body"].as_str().is_some_and(|b| !b.is_empty()));
    }
}
