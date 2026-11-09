//! The delivery planner's server side (P10): asks the fast-tier model for a turn's plan before the
//! bot starts ([`plan_for_message`]), keeps it for the turns of that job (the answering bot and the
//! teammates it hands parts to, [`remember`] / [`planned`]), and learns from what the person says
//! next ([`note_reaction`]).
//!
//! Which model plans: the Claude API key when the space has one (fast, well under the 3 s limit);
//! the Claude CLI only when the `delivery_planner` setting is `cli` (a CLI start alone can take
//! seconds); none when it is `off`. Without a model every turn keeps the keyword plan.

use crate::db::models::{Kwant, Message};
use crate::db::spaces::SpaceId;
use crate::router::analyzer::{self, Bot, Input, Note, Outcome, Why};
use crate::router::delivery::{keyword_plan, Deliverable, DeliveryPlan, Show};
use crate::server::AppState;
use crate::zombie::classify::{ApiClassifier, CliClassifier, Classifier, DEFAULT_API_MODEL};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The setting that turns the planner off (`off`) or lets it use the Claude CLI (`cli`).
pub const SETTING: &str = "delivery_planner";
/// The reply's meta key for the plan it was made with. Not `delivery`: that is the delivery
/// check's verdict (`delivery_check::meta`, #469).
pub const META_KEY: &str = "delivery_plan";
/// How long a job's plan is kept at most.
const PLAN_TTL: Duration = Duration::from_secs(60 * 60);
/// Plans kept at most (the oldest go first).
const MAX_PLANS: usize = 256;
/// Earlier messages the planner reads.
const RECENT_MESSAGES: i64 = 12;
/// README bytes read for the project's kind.
const README_BYTES: usize = 4096;

fn plans() -> &'static Mutex<HashMap<String, (Instant, DeliveryPlan)>> {
    static PLANS: OnceLock<Mutex<HashMap<String, (Instant, DeliveryPlan)>>> = OnceLock::new();
    PLANS.get_or_init(Default::default)
}

/// Keeps `plan` for the turns of job `job_id`.
pub fn remember(job_id: &str, plan: DeliveryPlan) {
    let Ok(mut map) = plans().lock() else { return };
    map.retain(|_, (at, _)| at.elapsed() < PLAN_TTL);
    while map.len() >= MAX_PLANS {
        let Some(oldest) = map.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) else { break };
        map.remove(&oldest);
    }
    map.insert(job_id.to_string(), (Instant::now(), plan));
}

/// The plan kept for job `job_id`, if any.
pub fn planned(job_id: &str) -> Option<DeliveryPlan> {
    let map = plans().lock().ok()?;
    map.get(job_id).filter(|(at, _)| at.elapsed() < PLAN_TTL).map(|(_, p)| p.clone())
}

/// Drops job `job_id`'s plan when its turns are done.
pub fn forget(job_id: &str) {
    if let Ok(mut map) = plans().lock() {
        map.remove(job_id);
    }
}

/// The planner's model, or `None` when it is off or the space has no Claude account it may use.
pub async fn planner_model(state: &AppState) -> Option<Box<dyn Classifier>> {
    let setting = state.db.get_setting(SETTING).await.ok().flatten().unwrap_or_default().trim().to_ascii_lowercase();
    if matches!(setting.as_str(), "off" | "false" | "0" | "no") {
        return None;
    }
    let key = state.db.get_setting(crate::profiles::Engine::Claude.key_setting()).await.ok().flatten();
    use crate::drivers::engines::SystemRoute;
    match crate::drivers::engines::system_route(&state.engines, DEFAULT_API_MODEL, key, crate::drivers::find_cli_binary) {
        Ok(SystemRoute::Api { engine, key }) if checked_ready(state.engines.status(engine)) => {
            Some(Box::new(ApiClassifier::new("https://api.anthropic.com", key, DEFAULT_API_MODEL)))
        }
        Ok(SystemRoute::Cli { engine, bin }) if setting == "cli" && checked_ready(state.engines.status(engine)) => {
            Some(Box::new(CliClassifier::new(&bin, DEFAULT_API_MODEL)))
        }
        _ => None,
    }
}

/// The planner asks a model only once the engine check found its account working: not while the
/// check is still running (a key that is only set may be wrong, and 3 s of every turn would go to
/// it), and so never in tests, which set fake keys and never pass the check.
pub fn checked_ready(status: crate::drivers::engines::EngineStatus) -> bool {
    status == crate::drivers::engines::EngineStatus::Ready
}

/// Records what the delivery check (Vega's `delivery_check::record_delivery_note`, #469) found
/// after a turn of `kwant_id` in chat `context_id`: what the plan wanted the person to see, whether
/// all of it was delivered, and what was missing. Kept as a `check` note next to the person's own
/// (`person`) notes; the planner learns only from the person's notes, so a bot's miss never reads
/// as the person's taste. Errors are logged, never returned: the reply is saved either way.
pub async fn record_check(state: &AppState, kwant_id: &str, context_id: &str, wanted: &[Show], hit: bool, missing: &[Show]) {
    let Some(note) = check_note(wanted, hit, missing) else { return };
    let space = match state.db.get_kwant(kwant_id).await {
        Ok(Some(k)) => SpaceId::new(&k.space_id),
        _ => return,
    };
    if let Err(e) = state.db.add_delivery_check(&space, kwant_id, context_id, &note).await {
        tracing::warn!(error = %e, "could not save a delivery check note");
    }
}

/// The check note for [`record_check`]: `None` when the plan wanted nothing the check can see.
pub fn check_note(wanted: &[Show], hit: bool, missing: &[Show]) -> Option<Note> {
    let kind = check_kind(wanted)?;
    let shown: Vec<Show> = wanted.iter().copied().filter(|w| !missing.contains(w)).collect();
    Some(Note { kind, outcome: if hit { Outcome::Hit } else { Outcome::Miss }, wanted: missing.first().copied(), shown })
}

/// The kind a check note is filed under: a picture the person expected is a visual result, a link a
/// change. `None` when the plan wanted neither (nothing was checked).
pub fn check_kind(wanted: &[Show]) -> Option<Deliverable> {
    if wanted.contains(&Show::Link) {
        Some(Deliverable::Change)
    } else if wanted.contains(&Show::Image) {
        Some(Deliverable::Visual)
    } else {
        None
    }
}

/// The chat's earlier turns for the planner, oldest first, without the message being planned:
/// ("person" or the bot's name, text), and the person's own earlier messages for the keyword plan.
pub fn recent_turns(history: &[Message], prompt: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut msgs: Vec<&Message> = history.iter().filter(|m| m.sender_role == "user" || m.sender_role == "kwant").collect();
    if msgs.last().is_some_and(|m| m.sender_role == "user" && m.text.trim() == prompt.trim()) {
        msgs.pop();
    }
    let turns = msgs
        .iter()
        .filter(|m| !m.text.trim().is_empty())
        .map(|m| (if m.sender_role == "user" { "person".to_string() } else { m.sender_id.clone() }, m.text.clone()))
        .collect();
    let person = msgs.iter().filter(|m| m.sender_role == "user").map(|m| m.text.clone()).collect();
    (turns, person)
}

/// What the project is, from the README in `workspace` (empty without one).
fn project_of(workspace: Option<&std::path::Path>) -> String {
    let Some(dir) = workspace else { return String::new() };
    for name in ["README.md", "readme.md", "README", "README.txt"] {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            let head = &bytes[..bytes.len().min(README_BYTES)];
            return analyzer::project_kind(&String::from_utf8_lossy(head));
        }
    }
    String::new()
}

/// The space's newest delivery notes, for the planner.
pub async fn notes(state: &AppState, space: &SpaceId) -> Vec<Note> {
    let rows = state.db.list_person_delivery_notes(space, analyzer::MAX_NOTES as i64).await.unwrap_or_default();
    rows.iter().filter_map(|r| r.note()).collect()
}

/// The plan for the person's message `prompt` to `answering` in chat `chat_id`: the model's when it
/// answers in time, else the keyword plan, with the person's notes applied. In a team chat the bot
/// the chat routed the message to is the one that replies ([`analyzer::align_replier`]).
pub async fn plan_for_message(
    state: &AppState,
    answering: &Kwant,
    teammates: &[Kwant],
    chat_id: &str,
    workspace: Option<&std::path::Path>,
    prompt: &str,
) -> (DeliveryPlan, Why) {
    let space = SpaceId::new(&answering.space_id);
    let history = state.db.list_messages(chat_id, RECENT_MESSAGES).await.unwrap_or_default();
    let (recent, person) = recent_turns(&history, prompt);
    let person: Vec<&str> = person.iter().map(String::as_str).collect();
    let keyword = keyword_plan(prompt, &person);
    let mut team = vec![Bot { id: answering.id.clone(), name: answering.name.clone(), title: answering.title.clone() }];
    for k in teammates.iter().filter(|k| k.id != answering.id) {
        team.push(Bot { id: k.id.clone(), name: k.name.clone(), title: k.title.clone() });
    }
    let input = Input { message: prompt.to_string(), recent, team, project: project_of(workspace), notes: notes(state, &space).await };
    let model = if keyword.kind == crate::router::delivery::Deliverable::Other { None } else { planner_model(state).await };
    let (mut plan, why) = analyzer::plan(model.as_deref(), &input, keyword, analyzer::timeout()).await;
    analyzer::align_replier(&mut plan, &answering.id);
    tracing::info!(kwant = %answering.id, kind = ?plan.kind, source = ?plan.source, why = why.as_str(), "delivery plan");
    (plan, why)
}

/// The reply the person's newest message reacts to, and the note it earns: the bots' messages
/// since the person's previous message are what they were shown; the reply's plan is the one kept
/// in its meta (else the keyword plan of the message it answered). `None` when there is nothing
/// to judge. `history` is oldest first and ends with the person's new message.
pub fn judge_chat(history: &[Message]) -> Option<(String, Option<String>, Note)> {
    let (now, before) = history.split_last()?;
    if now.sender_role != "user" {
        return None;
    }
    let asked_at = before.iter().rposition(|m| m.sender_role == "user")?;
    let asked = &before[asked_at];
    let replies: Vec<&Message> = before[asked_at + 1..].iter().filter(|m| m.sender_role == "kwant").collect();
    let answer = replies.iter().rev().find(|m| !m.text.trim().is_empty() && meta_kind(m).is_none())?;
    let plan = meta(answer)
        .and_then(|m| serde_json::from_value::<DeliveryPlan>(m.get(META_KEY)?.clone()).ok())
        .unwrap_or_else(|| keyword_plan(&asked.text, &[]));
    let text: Vec<&str> = replies.iter().map(|m| m.text.as_str()).collect();
    let mimes: Vec<String> = replies.iter().flat_map(|m| attachment_mimes(m)).collect();
    let mimes: Vec<&str> = mimes.iter().map(String::as_str).collect();
    let shown = analyzer::shown_in(&text.join("\n"), &mimes);
    let repeated = crate::router::team::alike(
        &crate::router::team::content_words(&now.text),
        &crate::router::team::content_words(&asked.text),
    );
    let note = analyzer::judge(&now.text, repeated, &plan, &shown)?;
    Some((answer.id.clone(), answer.direct_kwant_id.clone(), note))
}

fn meta(m: &Message) -> Option<serde_json::Value> {
    serde_json::from_str(m.meta.as_deref()?).ok()
}

fn meta_kind(m: &Message) -> Option<String> {
    meta(m)?.get("kind")?.as_str().map(str::to_string)
}

fn attachment_mimes(m: &Message) -> Vec<String> {
    let Some(meta) = meta(m) else { return Vec::new() };
    meta.get("attachments")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("mime").and_then(|v| v.as_str()).map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Notes how the reply before the person's new message `text` in chat `chat_id` went down (see
/// [`judge_chat`]); at most one note per reply.
pub async fn note_reaction(state: &AppState, space: &SpaceId, chat_id: &str) {
    let history = state.db.list_messages(chat_id, 20).await.unwrap_or_default();
    let Some((message_id, kwant_id, note)) = judge_chat(&history) else { return };
    let person = history.last().map(|m| m.text.as_str()).unwrap_or_default();
    if let Err(e) = state.db.add_delivery_note(space, kwant_id.as_deref(), &message_id, &note, person).await {
        tracing::warn!(error = %e, "could not save a delivery note");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::analyzer::Outcome;
    use crate::router::delivery::{Deliverable, Show};

    fn msg(role: &str, text: &str, meta: Option<serde_json::Value>) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            sender_role: role.into(),
            sender_id: if role == "user" { "user".into() } else { "Frontend".into() },
            text: text.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
            created_at: None,
            thread_id: None,
            meta: meta.map(|m| m.to_string()),
            results: None,
            reply_to_message_id: None,
            reaction: None,
            reply_to: None,
        }
    }

    #[test]
    fn show_me_after_a_text_only_reply_is_noted_as_a_miss() {
        let reply = msg("kwant", "It's the bar at the bottom of the mobile page that opens booking.", None);
        let chat = vec![
            msg("user", "mobile sticky booking CTA -> what is this??", None),
            reply.clone(),
            msg("user", "show me", None),
        ];
        let (id, kwant, note) = judge_chat(&chat).unwrap();
        assert_eq!(id, reply.id);
        assert_eq!(kwant.as_deref(), Some("k1"));
        assert_eq!(note, Note { kind: Deliverable::Explain, outcome: Outcome::Miss, wanted: Some(Show::Image), shown: vec![Show::Text] });
    }

    #[test]
    fn a_shared_image_then_moving_on_is_a_hit_and_the_replys_own_plan_is_used() {
        let plan = keyword_plan("Screenshot give me!", &[]);
        let reply = msg("kwant", "Here it is.", Some(serde_json::json!({ (META_KEY): plan, "delivery": { "outcome": "hit" } })));
        let file = msg("kwant", "home.png", Some(serde_json::json!({ "kind": "file", "attachments": [{ "id": "a", "name": "home.png", "mime": "image/png", "size": 1 }] })));
        let chat = vec![msg("user", "how does it look now?", None), reply.clone(), file, msg("user", "great, now the footer", None)];
        let (id, _, note) = judge_chat(&chat).unwrap();
        assert_eq!(id, reply.id, "the file message isn't the reply judged");
        assert_eq!(note.kind, Deliverable::Visual, "the reply's own plan, not the keyword plan of its ask");
        assert_eq!(note.outcome, Outcome::Hit);
        assert!(note.shown.contains(&Show::Image));
    }

    #[test]
    fn repeating_the_ask_is_a_miss_and_nothing_is_judged_without_a_reply() {
        let chat = vec![msg("user", "Fix the differences on the pricing page", None), msg("kwant", "I can push a fix if you like.", None), msg("user", "Fix the differences on the pricing page!!", None)];
        let (_, _, note) = judge_chat(&chat).unwrap();
        assert_eq!((note.outcome, note.wanted), (Outcome::Miss, Some(Show::Link)));
        assert!(judge_chat(&[msg("user", "hi", None), msg("user", "show me", None)]).is_none());
        assert!(judge_chat(&[msg("user", "show me", None)]).is_none());
        assert!(judge_chat(&[msg("user", "hi", None), msg("kwant", "Hello!", None), msg("user", "show me", None)]).is_none(), "small talk");
    }

    #[test]
    fn the_planner_reads_earlier_turns_without_the_message_itself() {
        let chat = vec![msg("user", "what is this??", None), msg("kwant", "A bar.", None), msg("user", "show me", None)];
        let (turns, person) = recent_turns(&chat, "show me");
        assert_eq!(turns, vec![("person".to_string(), "what is this??".to_string()), ("Frontend".to_string(), "A bar.".to_string())]);
        assert_eq!(person, vec!["what is this??".to_string()]);
    }

    #[test]
    fn the_planner_asks_a_model_only_once_its_account_checked_ready() {
        use crate::drivers::engines::EngineStatus;
        assert!(checked_ready(EngineStatus::Ready));
        // A key that is only set (tests, the first seconds after start) is never asked
        for s in [EngineStatus::Checking, EngineStatus::Error, EngineStatus::SignedOut, EngineStatus::NotInstalled, EngineStatus::Disabled] {
            assert!(!checked_ready(s), "{s:?}");
        }
    }

    #[test]
    fn a_check_note_is_filed_by_what_the_plan_wanted() {
        assert_eq!(check_kind(&[Show::Image]), Some(Deliverable::Visual));
        assert_eq!(check_kind(&[Show::Link, Show::Image]), Some(Deliverable::Change));
        assert_eq!(check_kind(&[Show::Text]), None);
    }

    #[test]
    fn a_check_note_names_what_was_missing() {
        let miss = check_note(&[Show::Image], false, &[Show::Image]).unwrap();
        assert_eq!(miss, Note { kind: Deliverable::Visual, outcome: Outcome::Miss, wanted: Some(Show::Image), shown: vec![] });
        let hit = check_note(&[Show::Link, Show::Image], true, &[]).unwrap();
        assert_eq!((hit.kind, hit.outcome, hit.wanted), (Deliverable::Change, Outcome::Hit, None));
        assert_eq!(hit.shown, vec![Show::Link, Show::Image]);
        assert!(check_note(&[Show::Text], true, &[]).is_none(), "nothing to check, nothing noted");
    }

    #[test]
    fn a_jobs_plan_is_kept_for_its_turns_and_dropped_after() {
        let job = format!("job-{}", uuid::Uuid::new_v4());
        assert!(planned(&job).is_none());
        remember(&job, keyword_plan("show me mockup image", &[]));
        assert_eq!(planned(&job).unwrap().kind, Deliverable::Visual);
        forget(&job);
        assert!(planned(&job).is_none());
    }
}
