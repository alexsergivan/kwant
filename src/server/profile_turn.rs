//! Which profile a kwant turn runs on, the driver for it, and the chat notices when a turn
//! moves to another profile or every profile is out.

use super::AppState;
use crate::db::models::LlmProfile;
use crate::drivers::{find_cli_binary, resolve_driver_for_model, ApiDriver, CliDriver, Driver};
use crate::profiles::{
    failover_order, Availability, Engine, FailClass, ProfileStatus, ProviderFailure,
};

/// Profiles to try for a turn on `engine`, in order. Empty when the model has no profiles
/// (e.g. Ollama) or the engine has none enabled: the turn then runs as before profiles.
pub async fn plan(state: &AppState, engine: Option<Engine>, kwant_id: &str) -> Vec<LlmProfile> {
    let Some(engine) = engine else {
        return Vec::new();
    };
    let db = &state.db;
    let (Ok(profiles), Ok(binding), Ok(states), Ok(with_key)) = (
        db.list_profiles().await,
        db.get_kwant_binding(kwant_id).await,
        db.list_profile_states().await,
        db.profile_ids_with_key().await,
    ) else {
        return Vec::new();
    };
    let (bound, failover) = binding.unwrap_or((None, true));
    let avail = Availability {
        states,
        with_key,
        signed_out: state.profiles.signed_out(),
    };
    failover_order(
        &profiles,
        engine,
        bound.as_deref(),
        failover,
        &avail,
        chrono::Utc::now(),
    )
}

/// The driver for one attempt. `None` or the system profile: exactly the pre-profiles choice.
pub async fn driver_for(
    state: &AppState,
    profile: Option<&LlmProfile>,
    target_model: &str,
    configured_key: Option<String>,
    endpoint: Option<String>,
) -> Box<dyn Driver> {
    let Some(p) = profile.filter(|p| p.kind != "system") else {
        return resolve_driver_for_model(target_model, configured_key, endpoint);
    };
    let engine = Engine::parse(&p.engine).unwrap_or(Engine::Claude);
    if p.kind == "api_key" {
        let key = state
            .db
            .get_profile_key(&p.id)
            .await
            .ok()
            .flatten()
            .map(|k| k.0);
        return Box::new(ApiDriver::for_profile(key));
    }
    let home = state.profiles.home_of(p).unwrap_or_default();
    Box::new(
        CliDriver::new(
            find_cli_binary(engine.as_str()),
            Some(engine.as_str().into()),
        )
        .with_profile_home(engine, home),
    )
}

/// Remembers a refusal so failover skips the profile: until its quota resets, or until it is
/// signed in again.
pub async fn record_failure(state: &AppState, profile: &LlmProfile, failure: &ProviderFailure) {
    let until = (failure.class == FailClass::Quota)
        .then_some(failure.retry_at)
        .flatten();
    let _ = state
        .db
        .record_profile_failure(&profile.id, failure.class.as_str(), until)
        .await;
    if failure.class == FailClass::Auth && profile.kind != "api_key" {
        state.profiles.set_status(
            &profile.id,
            ProfileStatus {
                signed_in: Some(false),
                account_label: None,
            },
        );
    }
}

fn reason(failure: &ProviderFailure) -> String {
    match (failure.class, failure.retry_at) {
        (FailClass::Quota, Some(at)) => format!("is out of usage until {} UTC", at.format("%H:%M")),
        (FailClass::Quota, None) => "is out of usage".into(),
        _ => "is signed out or its key was rejected".into(),
    }
}

/// Shown while the turn moves on: "Work ran out of usage until 17:00 UTC; switching to Personal."
pub fn switch_notice(from: &LlmProfile, to: &LlmProfile, failure: &ProviderFailure) -> String {
    format!(
        "🔁 {} profile \"{}\" {}; switching to \"{}\".",
        from.engine,
        from.name,
        reason(failure),
        to.name
    )
}

/// The reply when every profile tried refused the turn.
pub fn give_up_notice(
    kwant_name: &str,
    last: &LlmProfile,
    failure: &ProviderFailure,
    tried: usize,
) -> String {
    let fix = match failure.class {
        FailClass::Quota => "Add another profile or wait for the limit to reset.",
        _ => "Sign in again (or update the key) under Settings → Profiles.",
    };
    let which = if tried > 1 {
        format!(
            "All {tried} {} profiles tried are unavailable; the last, \"{}\",",
            last.engine, last.name
        )
    } else {
        format!("The {} profile \"{}\"", last.engine, last.name)
    };
    format!("⚠️ [{kwant_name}] {which} {}. {fix}", reason(failure))
}
