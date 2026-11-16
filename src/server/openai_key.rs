//! Which OpenAI key a space uses. One order for hybrid recall's embeddings
//! (`memory_recall::source_for_space`) and dictation (`transcribe`), so the two can't drift:
//!
//! 1. an enabled OpenAI API-key account of the space (an `api_key` profile on Codex) with a key;
//! 2. the host's OpenAI key (Settings, else `OPENAI_API_KEY`), unless This machine is off for
//!    Codex in the space;
//!
//! and neither while the OpenAI API engine is turned off. Recall then falls back to a local
//! Ollama; dictation has nothing after OpenAI.

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::drivers::engines::EngineId;
use crate::profiles::Engine;

/// What decides a space's OpenAI key, read from the database and Settings.
#[derive(Debug, Default, Clone)]
pub struct KeyFacts {
    /// The OpenAI API engine is on (Settings).
    pub openai_on: bool,
    /// The key of the space's first enabled Codex `api_key` account that has one.
    pub space_key: Option<String>,
    /// This machine is off for Codex in the space.
    pub host_off: bool,
    /// The host's key (Settings, else `OPENAI_API_KEY`).
    pub host_key: Option<String>,
}

/// Why a space has no OpenAI key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoKey {
    /// The OpenAI API engine is off, and the space or the host has a key it would use.
    EngineOff,
    /// Only the host has a key, and This machine is off for Codex in the space.
    HostOff,
    /// No key anywhere.
    Missing,
}

/// The key to use (trimmed), or why there is none. Pure: the order lives here.
pub fn pick(facts: &KeyFacts) -> Result<String, NoKey> {
    let usable = |k: &Option<String>| k.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    if !facts.openai_on {
        return Err(if usable(&facts.space_key).is_some() || usable(&facts.host_key).is_some() { NoKey::EngineOff } else { NoKey::Missing });
    }
    if let Some(k) = usable(&facts.space_key) {
        return Ok(k);
    }
    match usable(&facts.host_key) {
        Some(_) if facts.host_off => Err(NoKey::HostOff),
        Some(k) => Ok(k),
        None => Err(NoKey::Missing),
    }
}

/// Reads [`KeyFacts`] for `space`: only the database, the environment and the cached engine
/// checks, never a network call.
pub async fn facts(state: &AppState, space: &SpaceId) -> KeyFacts {
    let db = &state.db;
    let profiles = db.list_profiles(space).await.unwrap_or_default();
    let mut space_key = None;
    for p in profiles.iter().filter(|p| p.engine == "codex" && p.kind == "api_key" && p.enabled) {
        if let Ok(Some(key)) = db.get_profile_key(&p.id).await {
            if !key.0.trim().is_empty() {
                space_key = Some(key.0);
                break;
            }
        }
    }
    let host_off = profiles.iter().any(|p| p.kind == "system" && !p.enabled && Engine::parse(&p.engine) == Some(Engine::Codex));
    let host_key = db.get_setting("openai_api_key").await.ok().flatten().or_else(|| std::env::var("OPENAI_API_KEY").ok()).filter(|k| !k.trim().is_empty());
    KeyFacts { openai_on: state.engines.is_enabled(EngineId::OpenaiApi), space_key, host_off, host_key }
}

/// The space's OpenAI key, or why there is none.
pub async fn key_for_space(state: &AppState, space: &SpaceId) -> Result<String, NoKey> {
    pick(&facts(state, space).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(openai_on: bool, space_key: Option<&str>, host_off: bool, host_key: Option<&str>) -> KeyFacts {
        KeyFacts { openai_on, space_key: space_key.map(Into::into), host_off, host_key: host_key.map(Into::into) }
    }

    /// The whole order in one table: recall and dictation both read it from here.
    #[test]
    fn the_order() {
        let cases: [(KeyFacts, Result<&str, NoKey>); 9] = [
            (facts(true, Some("sk-space"), false, Some("sk-host")), Ok("sk-space")),
            (facts(true, Some(" sk-space \n"), true, None), Ok("sk-space")),
            (facts(true, Some("sk-space"), true, Some("sk-host")), Ok("sk-space")),
            (facts(true, None, false, Some(" sk-host ")), Ok("sk-host")),
            (facts(true, None, true, Some("sk-host")), Err(NoKey::HostOff)),
            (facts(true, Some("  "), true, Some("sk-host")), Err(NoKey::HostOff)),
            (facts(true, Some(""), false, Some(" ")), Err(NoKey::Missing)),
            (facts(false, Some("sk-space"), false, Some("sk-host")), Err(NoKey::EngineOff)),
            (facts(false, None, false, None), Err(NoKey::Missing)),
        ];
        for (f, want) in cases {
            assert_eq!(pick(&f), want.map(str::to_string), "{f:?}");
        }
    }
}
