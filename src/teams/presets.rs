//! The bot presets the server builds bots from (spec D3/D5): a copy of the app's
//! `frontend/src/agentPresets.ts`, generated into `presets.json` by `scripts/gen-team-data.mjs`.
//! `frontend/tests/agentPresets.test.mjs` fails when the two differ.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub title: String,
    pub avatar_sprite: String,
    pub persona_prompt: String,
    pub soul: String,
}

pub fn all() -> &'static [Preset] {
    static ALL: OnceLock<Vec<Preset>> = OnceLock::new();
    ALL.get_or_init(|| serde_json::from_str(include_str!("presets.json")).expect("src/teams/presets.json is valid"))
}

pub fn get(id: &str) -> Option<&'static Preset> {
    all().iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_ids_are_unique() {
        let ids: std::collections::HashSet<_> = all().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), all().len());
        for id in ["frontend", "backend", "qa", "sre", "data", "writer", "security", "designer", "support", "copywriter", "research", "sales", "assistant"] {
            assert!(get(id).is_some(), "{id}");
        }
    }
}
