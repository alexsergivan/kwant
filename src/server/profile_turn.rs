//! Which profile a kwant turn runs on, the driver for it, and the chat notices when a turn
//! moves to another profile or every profile is out.

use super::AppState;
use crate::db::models::LlmProfile;
use crate::db::spaces::SpaceId;
use crate::drivers::engines::{EngineAccount, EngineId, EngineReport, EngineStatus};
use crate::drivers::{find_cli_binary, resolve_driver_for_model, ApiDriver, CliDriver, Driver};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::profiles::{
    failover_order, preference_order, Availability, Engine, FailClass, ProfileStatus,
    ProviderFailure, SignInBar,
};

/// The space's profiles and what decides whether each can take a turn now, from the db and the
/// cached sign-in checks (never waits on a check). `None` when the db can't be read.
async fn accounts(state: &AppState, space: &SpaceId) -> Option<(Vec<LlmProfile>, Availability)> {
    let db = &state.db;
    let (Ok(profiles), Ok(states), Ok(with_key)) = (
        db.list_profiles(space).await,
        db.list_profile_states().await,
        db.profile_ids_with_key().await,
    ) else {
        return None;
    };
    let avail = Availability {
        states,
        with_key,
        signed_out: state.profiles.signed_out(),
        signed_in: state.profiles.signed_in(),
    };
    Some((profiles, avail))
}

/// Profiles of the kwant's space to try for a turn on `engine`, in order. Empty when the model
/// has no profiles (e.g. Ollama) or the engine has none enabled: the turn then runs on the
/// host, unless [`host_off`] says This machine is off in that space.
pub async fn plan(state: &AppState, engine: Option<Engine>, kwant_id: &str) -> Vec<LlmProfile> {
    let Some(engine) = engine else {
        return Vec::new();
    };
    let Ok(Some(space)) = state.db.space_of_kwant(kwant_id).await else {
        return Vec::new();
    };
    let (Some((profiles, avail)), Ok(binding)) =
        (accounts(state, &space).await, state.db.get_kwant_binding(kwant_id).await)
    else {
        return Vec::new();
    };
    let (bound, failover) = binding.unwrap_or((None, true));
    failover_order(
        &profiles,
        engine,
        bound.as_deref(),
        failover,
        &avail,
        chrono::Utc::now(),
    )
    .into_iter()
    .filter(|p| profile_engine_on(state, p))
    .collect()
}

/// Engines whose This machine profile is off in `space`. A turn on one of them never falls back
/// to the host's own login or Settings key there; it needs one of the space's own accounts.
pub async fn host_off(state: &AppState, space: &SpaceId) -> HashSet<Engine> {
    let Ok(profiles) = state.db.list_profiles(space).await else {
        return HashSet::new();
    };
    profiles
        .iter()
        .filter(|p| p.kind == "system" && !p.enabled)
        .filter_map(|p| Engine::parse(&p.engine))
        .collect()
}

/// The engine a profile's turns run on: its family's API for a key, else its CLI. The system
/// profile runs on whichever host engine routing picks, so it has none of its own.
pub fn profile_engine(p: &LlmProfile) -> Option<EngineId> {
    let family = Engine::parse(&p.engine)?;
    let (cli, api) = EngineId::of_family(family);
    match p.kind.as_str() {
        "system" => None,
        "api_key" => Some(api),
        _ => Some(cli),
    }
}

/// The account a turn on `engine` runs on, for its "Running on" notice: the profile's name,
/// else "This machine" for a CLI's own login. API keys from Settings and Ollama have none.
pub fn account_name(profile: Option<&LlmProfile>, engine: EngineId) -> Option<String> {
    match profile {
        Some(p) => Some(p.name.clone()),
        None => (engine.kind() == crate::drivers::engines::EngineKind::Cli).then(|| "This machine".into()),
    }
}

/// An engine turned off in Settings is off for every account on it too.
fn profile_engine_on(state: &AppState, p: &LlmProfile) -> bool {
    profile_engine(p).is_none_or(|e| state.engines.is_enabled(e))
}

/// The driver for one attempt, and the engine it runs on when it's the host's own (no profile
/// or the system profile; see [`resolve_driver_for_model`]). `Err` when that engine can't run.
pub async fn driver_for(
    state: &AppState,
    profile: Option<&LlmProfile>,
    target_model: &str,
    configured_key: Option<String>,
    endpoint: Option<String>,
) -> Result<(Box<dyn Driver>, Option<EngineId>), String> {
    let Some(p) = profile.filter(|p| p.kind != "system") else {
        return resolve_driver_for_model(target_model, configured_key, endpoint, &state.engines)
            .map(|(d, e)| (d, Some(e)));
    };
    let engine = Engine::parse(&p.engine).unwrap_or(Engine::Claude);
    if p.kind == "custom_api" {
        let (endpoint, key) = custom_endpoint(state, p).await?;
        let protocol = crate::drivers::custom::Protocol::parse(&endpoint.protocol)
            .ok_or_else(|| format!("Custom API account \"{}\" has an unknown protocol", p.name))?;
        let chat = crate::drivers::custom::chat_url(&endpoint.base_url, protocol);
        return Ok((Box::new(ApiDriver::for_custom(key, chat, endpoint.base_url)), None));
    }
    if p.kind == "api_key" {
        let key = state
            .db
            .get_profile_key(&p.id)
            .await
            .ok()
            .flatten()
            .map(|k| k.0);
        return Ok((Box::new(ApiDriver::for_profile(key)), None));
    }
    let home = state.profiles.home_of(p).unwrap_or_default();
    Ok((
        Box::new(
            CliDriver::new(
                find_cli_binary(engine.as_str()),
                Some(engine.as_str().into()),
            )
            .with_profile_home(engine, home)
            .with_account(p.name.clone()),
        ),
        None,
    ))
}

/// A Custom API account's server and key. `Err` when its server was never saved.
pub async fn custom_endpoint(
    state: &AppState,
    p: &LlmProfile,
) -> Result<(crate::db::custom_api::ProfileEndpoint, Option<String>), String> {
    let endpoint = state
        .db
        .get_profile_endpoint(&p.id)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| format!("Custom API account \"{}\" has no server set", p.name))?;
    let key = state.db.get_profile_key(&p.id).await.ok().flatten().map(|k| k.0);
    Ok((endpoint, key))
}

/// Checks a Custom API account against its server (GET {base}/models) and caches the result.
/// Reuses a fresh check unless `force`.
pub async fn refresh_custom(
    state: &AppState,
    p: &LlmProfile,
    force: bool,
) -> Option<crate::drivers::custom::CustomCheck> {
    use crate::drivers::custom::{check, CheckStatus, CustomCheck, Protocol};
    if p.kind != "custom_api" {
        return None;
    }
    if !force {
        if let Some(c) = state.profiles.custom_check(&p.id) {
            return Some(c);
        }
    }
    let result = match custom_endpoint(state, p).await {
        Ok((endpoint, key)) => match Protocol::parse(&endpoint.protocol) {
            Some(proto) => check(proto, &endpoint.base_url, key.as_deref(), endpoint.model.as_deref()).await,
            None => CustomCheck::new(CheckStatus::Error, "Unknown protocol", Vec::new()),
        },
        Err(e) => CustomCheck::new(CheckStatus::Error, e, Vec::new()),
    };
    state.profiles.set_custom(&p.id, result.clone());
    Some(result)
}

/// The model a turn on a Custom API account runs: the bot's pinned model as is (the server
/// knows its own ids best), else the account's own model, else the one its server lists for the
/// tier. `None` when none is known; the driver then says so.
pub async fn custom_model(
    state: &AppState,
    p: &LlmProfile,
    pinned: Option<&str>,
    tier: crate::drivers::engines::Tier,
) -> Option<crate::drivers::engines::ModelChoice> {
    use crate::drivers::engines::{choose_model, ModelChoice};
    let named = |id: &str| ModelChoice {
        id: id.to_string(),
        label: id.to_string(),
        tier: crate::drivers::engines::tier_from_name(id),
    };
    if let Some(pin) = pinned.map(str::trim).filter(|m| !m.is_empty()) {
        return Some(named(pin));
    }
    let stored = state.db.get_profile_endpoint(&p.id).await.ok().flatten().and_then(|e| e.model);
    if let Some(m) = stored {
        return Some(named(&m));
    }
    let models = refresh_custom(state, p, false).await.map(|c| c.models).unwrap_or_default();
    choose_model(&models, None, tier)
}

/// `family`'s own accounts (not the system profile) that pass `bar` and whose engine is on,
/// in the order failover prefers them. The one rule for "this account can take a turn".
fn own_accounts<'a>(
    state: &AppState,
    profiles: &'a [LlmProfile],
    family: Engine,
    avail: &Availability,
    bar: SignInBar,
) -> Vec<&'a LlmProfile> {
    let now = chrono::Utc::now();
    preference_order(profiles, family, None)
        .into_iter()
        .filter(|p| p.kind != "system" && avail.admits(p, now, bar))
        .filter(|p| profile_engine_on(state, p))
        .collect()
}

/// Engines with an enabled account of their own (not the system profile) that failover
/// would use: these engines can run even when the host's own CLI or key can't. Routing keeps
/// the lenient bar (an unchecked profile counts) so turns right after startup, before any
/// sign-in check, still reach a profile that is in fact signed in.
pub async fn profile_engines(state: &AppState, space: &SpaceId) -> HashSet<Engine> {
    let Some((profiles, avail)) = accounts(state, space).await else {
        return HashSet::new();
    };
    [Engine::Claude, Engine::Codex, Engine::Grok, Engine::Custom]
        .into_iter()
        .filter(|f| !own_accounts(state, &profiles, *f, &avail, SignInBar::NotSignedOut).is_empty())
        .collect()
}

/// Whether `report`'s engine is ready and the account a kwant on it uses: the first of the
/// engine's accounts in failover order (the default first) that can take a turn, the system
/// profile ("This machine") counting when the engine's own login is ready. Off ⇒ not ready, and
/// This machine turned off means the host's own login doesn't count either.
pub fn readiness(
    report: &EngineReport,
    profiles: &[LlmProfile],
    avail: &Availability,
    now: chrono::DateTime<chrono::Utc>,
) -> (bool, Option<EngineAccount>) {
    if !report.enabled {
        return (false, None);
    }
    let host = report.status == EngineStatus::Ready;
    let Some(family) = report.id.family() else {
        return (host, None);
    };
    let system = |p: &&LlmProfile| p.kind == "system";
    let host_off = profiles.iter().any(|p| system(&p) && p.engine == family.as_str() && !p.enabled);
    // Custom API runs only on the space's own accounts, never on anything of the host's
    let host = host && !host_off && family != Engine::Custom;
    let picked = preference_order(profiles, family, None)
        .into_iter()
        .filter(|p| system(p) || profile_engine(p) == Some(report.id))
        .find(|p| {
            if system(p) {
                host
            } else {
                avail.admits(p, now, SignInBar::SignedIn)
            }
        });
    let uses = picked.map(|p| EngineAccount { profile_id: p.id.clone(), label: p.name.clone() });
    (host || uses.is_some(), uses)
}

/// Fills in `ready` and `uses` on every report, for `space`'s accounts.
pub async fn with_accounts(state: &AppState, space: &SpaceId, mut reports: Vec<EngineReport>) -> Vec<EngineReport> {
    let accounts = accounts(state, space).await;
    let now = chrono::Utc::now();
    for r in &mut reports {
        let (ready, uses) = match &accounts {
            Some((profiles, avail)) => readiness(r, profiles, avail, now),
            None => (r.enabled && r.status == EngineStatus::Ready, None),
        };
        // An engine switched off is off for its accounts too (see profile_engine_on)
        r.ready = ready;
        r.uses = uses;
    }
    reports
}

/// How many of `space`'s enabled accounts could take a turn now: This machine when its
/// engine's own login is ready and the engine is on, any other account when it is signed in
/// and its engine is on.
pub async fn ready_accounts(state: &AppState, space: &SpaceId) -> usize {
    let Some((profiles, avail)) = accounts(state, space).await else {
        return 0;
    };
    let reports = state.engines.snapshot();
    let host_ready = |family: Engine| {
        let (cli, _) = EngineId::of_family(family);
        reports.iter().any(|r| r.id == cli && r.enabled && r.status == EngineStatus::Ready)
    };
    let now = chrono::Utc::now();
    profiles
        .iter()
        .filter(|p| p.enabled)
        .filter(|p| match (p.kind.as_str(), Engine::parse(&p.engine)) {
            ("system", Some(family)) => host_ready(family),
            ("system", None) => false,
            _ => avail.admits(p, now, SignInBar::SignedIn) && profile_engine_on(state, p),
        })
        .count()
}

/// The account a bot's next turn would run on, for its card: the first account failover would
/// try, else This machine when the engine's own login may be used there. `blocked` when This
/// machine is off in the space and no account of its own can take the turn. Bots on "auto" or
/// on a model without accounts (Ollama) have none.
pub async fn runs_on(state: &AppState, kwant: &crate::db::models::Kwant) -> serde_json::Value {
    let model = kwant.preferred_model.as_str();
    let engine = Engine::parse(model).or_else(|| Engine::from_model(model));
    let Some(engine) = engine else {
        return serde_json::json!({ "auto": model == "auto", "engine": null, "account": null, "profile_id": null, "blocked": false });
    };
    if let Some(p) = plan(state, Some(engine), &kwant.id).await.into_iter().next() {
        return serde_json::json!({ "auto": false, "engine": engine.as_str(), "account": p.name, "profile_id": p.id, "blocked": false });
    }
    let space = SpaceId::new(&kwant.space_id);
    let blocked = host_off(state, &space).await.contains(&engine);
    serde_json::json!({
        "auto": false,
        "engine": engine.as_str(),
        "account": if blocked { None } else { Some("This machine") },
        "profile_id": null,
        "blocked": blocked,
    })
}

static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Runs the stale profile sign-in checks so `ready` is current; one run at a time, others
/// return at once.
pub async fn refresh_sign_ins(state: &AppState) {
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    for space in state.db.list_spaces().await.unwrap_or_default() {
        if let Ok(profiles) = state.db.list_profiles(&SpaceId::new(&space.id)).await {
            let keyed = super::profile_routes::keyed_ids(state, &profiles).await;
            state.profiles.statuses(&profiles, &keyed).await;
            for p in profiles.iter().filter(|p| p.kind == "custom_api" && p.enabled) {
                refresh_custom(state, p, false).await;
            }
        }
    }
    REFRESHING.store(false, Ordering::Release);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::LlmProfileState;
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use std::collections::HashMap;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap()
    }

    fn profile(id: &str, name: &str, kind: &str, priority: i64, default: bool) -> LlmProfile {
        LlmProfile {
            id: id.into(),
            engine: "claude".into(),
            name: name.into(),
            kind: kind.into(),
            home_dir: None,
            priority,
            enabled: true,
            is_default: default,
            created_at: None,
            space_id: "spc_default".into(),
        }
    }

    fn report(id: EngineId, enabled: bool, status: EngineStatus) -> EngineReport {
        EngineReport {
            id,
            name: id.name().into(),
            kind: id.kind(),
            enabled,
            status,
            detail: None,
            version: None,
            models: Vec::new(),
            checked_at: None,
            ready: false,
            uses: None,
        }
    }

    fn signed_in(ids: &[&str]) -> Availability {
        Availability {
            signed_in: ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn uses(r: (bool, Option<EngineAccount>)) -> (bool, Option<String>) {
        (r.0, r.1.map(|a| a.label))
    }

    /// Oleks's setup: This machine plus a Personal Claude account that is the default.
    fn oleks() -> Vec<LlmProfile> {
        vec![
            profile("prf_system_claude", "This machine", "system", 0, false),
            profile("p_personal", "Personal", "cli_home", 100, true),
        ]
    }

    #[test]
    fn engine_off_is_not_ready_even_with_a_signed_in_account() {
        let r = report(EngineId::Claude, false, EngineStatus::Disabled);
        let avail = signed_in(&["p_personal"]);
        assert_eq!(readiness(&r, &oleks(), &avail, now()), (false, None));
    }

    #[test]
    fn host_signed_out_but_personal_signed_in_is_ready_on_personal() {
        let r = report(EngineId::Claude, true, EngineStatus::SignedOut);
        let got = readiness(&r, &oleks(), &signed_in(&["p_personal"]), now());
        assert!(got.0);
        assert_eq!(
            got.1,
            Some(EngineAccount { profile_id: "p_personal".into(), label: "Personal".into() })
        );
        // Host signed out and no account signed in: not ready
        let got = readiness(&r, &oleks(), &Availability::default(), now());
        assert_eq!(got, (false, None));
    }

    #[test]
    fn unknown_sign_in_is_not_ready_but_routing_still_tries_it() {
        let r = report(EngineId::Claude, true, EngineStatus::SignedOut);
        let unknown = Availability::default();
        assert_eq!(readiness(&r, &oleks(), &unknown, now()), (false, None));
        // Routing keeps the lenient bar: an unchecked profile is still tried by a turn
        let personal = &oleks()[1];
        assert!(unknown.admits(personal, now(), SignInBar::NotSignedOut));
        assert!(!unknown.admits(personal, now(), SignInBar::SignedIn));
    }

    #[test]
    fn quota_limited_account_is_not_ready_until_it_resets() {
        let r = report(EngineId::Claude, true, EngineStatus::SignedOut);
        let limited = |until: DateTime<Utc>| Availability {
            states: HashMap::from([(
                "p_personal".to_string(),
                LlmProfileState {
                    profile_id: "p_personal".into(),
                    limited_until: Some(until),
                    last_error: Some("quota".into()),
                },
            )]),
            ..signed_in(&["p_personal"])
        };
        let avail = limited(now() + Duration::hours(1));
        assert_eq!(readiness(&r, &oleks(), &avail, now()), (false, None));
        let avail = limited(now() - Duration::minutes(1));
        assert_eq!(uses(readiness(&r, &oleks(), &avail, now())), (true, Some("Personal".into())));
    }

    #[test]
    fn uses_the_default_first_then_priority_order() {
        let r = report(EngineId::Claude, true, EngineStatus::Ready);
        let profiles = vec![
            profile("prf_system_claude", "This machine", "system", 0, false),
            profile("p_work", "Work", "cli_home", 10, false),
            profile("p_personal", "Personal", "cli_home", 20, true),
        ];
        // The default wins over a higher priority, signed-in account and the host's own login
        let all = signed_in(&["p_work", "p_personal"]);
        assert_eq!(uses(readiness(&r, &profiles, &all, now())), (true, Some("Personal".into())));
        // Default not signed in: next by priority, here This machine (host ready)
        let work = signed_in(&["p_work"]);
        assert_eq!(uses(readiness(&r, &profiles, &work, now())), (true, Some("This machine".into())));
        // Host signed out too: Work
        let r = report(EngineId::Claude, true, EngineStatus::SignedOut);
        assert_eq!(uses(readiness(&r, &profiles, &work, now())), (true, Some("Work".into())));
    }

    #[test]
    fn disabled_profiles_and_other_engines_accounts_do_not_count() {
        let r = report(EngineId::Claude, true, EngineStatus::SignedOut);
        let mut profiles = oleks();
        profiles[1].enabled = false;
        assert_eq!(readiness(&r, &profiles, &signed_in(&["p_personal"]), now()), (false, None));
        // An API-key account runs on the Anthropic API, not the Claude CLI, and needs its key
        let profiles = vec![
            profile("prf_system_claude", "This machine", "system", 0, true),
            profile("p_key", "Team key", "api_key", 10, false),
        ];
        assert_eq!(readiness(&r, &profiles, &Availability::default(), now()), (false, None));
        let api = report(EngineId::AnthropicApi, true, EngineStatus::SignedOut);
        assert_eq!(readiness(&api, &profiles, &Availability::default(), now()), (false, None));
        let keyed = Availability {
            with_key: HashSet::from(["p_key".to_string()]),
            ..Default::default()
        };
        assert_eq!(uses(readiness(&api, &profiles, &keyed, now())), (true, Some("Team key".into())));
        assert_eq!(readiness(&r, &profiles, &keyed, now()), (false, None));
    }

    #[test]
    fn this_machine_off_is_never_a_fallback_even_when_the_host_is_ready() {
        let r = report(EngineId::Claude, true, EngineStatus::Ready);
        let mut profiles = oleks();
        profiles[0].enabled = false;
        assert_eq!(readiness(&r, &profiles, &Availability::default(), now()), (false, None));
        // Another account of the space still makes it ready
        assert_eq!(
            uses(readiness(&r, &profiles, &signed_in(&["p_personal"]), now())),
            (true, Some("Personal".into()))
        );
        // Custom API is ready only through a checked account of the space, never the host
        let c = report(EngineId::CustomApi, true, EngineStatus::Ready);
        let mut maestro = profile("p_maestro", "Maestro", "custom_api", 10, true);
        maestro.engine = "custom".into();
        assert_eq!(readiness(&c, &[], &Availability::default(), now()), (false, None));
        assert_eq!(readiness(&c, std::slice::from_ref(&maestro), &Availability::default(), now()), (false, None));
        assert_eq!(
            uses(readiness(&c, std::slice::from_ref(&maestro), &signed_in(&["p_maestro"]), now())),
            (true, Some("Maestro".into()))
        );
        // Ollama has no accounts: ready from its own check alone
        let o = report(EngineId::Ollama, true, EngineStatus::Ready);
        assert_eq!(readiness(&o, &profiles, &Availability::default(), now()), (true, None));
    }
}
