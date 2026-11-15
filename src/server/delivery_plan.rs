//! The delivery planner's server side (P10): asks the fast-tier model for a turn's plan before the
//! bot starts ([`plan_for_message`]), keeps it for the turn that answers the person's message
//! ([`remember`] / [`plan_for_answer`]), and learns from what the person says next
//! ([`note_reaction`]).
//!
//! The planner follows `delivery::plan_for_turn`'s rules (#470): it reads only the person's own
//! saved message and their earlier ones, never a turn's prompt (which carries kwant's routing,
//! hire and attachment notes); a prompt kwant or a bot wrote (a hand-off, routine, mission, decide
//! round) or a thanks-only message gets no plan and no planner call.
//!
//! Which model plans: the Claude API key when the space has one (fast, well under the 3 s limit);
//! the Claude CLI only when the `delivery_planner` setting is `cli` (a CLI start alone can take
//! seconds); none when it is `off`. Without a model every turn keeps the keyword plan.

use crate::db::models::{Kwant, Message};
use crate::db::spaces::SpaceId;
use crate::router::analyzer::{self, Bot, Input, Note, Outcome, Why};
use crate::router::delivery::{keyword_plan, Deliverable, DeliveryPlan, Show, TurnPlanInput};
use crate::server::routes::is_persons;
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

/// A job's plan: when it was made, the id of the person's message it was made from, the plan.
type Kept = (Instant, String, DeliveryPlan);

fn plans() -> &'static Mutex<HashMap<String, Kept>> {
    static PLANS: OnceLock<Mutex<HashMap<String, Kept>>> = OnceLock::new();
    PLANS.get_or_init(Default::default)
}

/// Keeps `plan`, made from the person's message `message_id`, for job `job_id`.
pub fn remember(job_id: &str, message_id: &str, plan: DeliveryPlan) {
    let Ok(mut map) = plans().lock() else { return };
    map.retain(|_, (at, _, _)| at.elapsed() < PLAN_TTL);
    while map.len() >= MAX_PLANS {
        let Some(oldest) = map.iter().min_by_key(|(_, (at, _, _))| *at).map(|(k, _)| k.clone()) else { break };
        map.remove(&oldest);
    }
    map.insert(job_id.to_string(), (Instant::now(), message_id.to_string(), plan));
}

/// Job `job_id`'s plan for a turn that answers the person's message `answering` (its id): only
/// the message the plan was made from. A hand-off, routine or decide-round turn of the same job
/// answers no saved message of the person and gets none.
pub fn plan_for_answer(job_id: &str, answering: Option<&str>) -> Option<DeliveryPlan> {
    let answering = answering?;
    let map = plans().lock().ok()?;
    map.get(job_id).filter(|(at, id, _)| at.elapsed() < PLAN_TTL && id == answering).map(|(_, _, p)| p.clone())
}

/// The person's own message a turn answers: the chat's `newest` message when it is the person's
/// (not a connected agent's nor a bot's chat-API message, [`is_persons`]) and `prompt` is that message, maybe with kwant's notes after it
/// (`drivers::drop_current_message`). `None` for a prompt kwant or a bot wrote: a hand-off, a
/// routine, a mission, a decide round.
pub fn answered_message<'a>(newest: Option<&'a Message>, prompt: &str) -> Option<&'a Message> {
    let m = newest.filter(|m| is_persons(m))?;
    let mut probe = vec![m.clone()];
    crate::drivers::drop_current_message(&mut probe, prompt);
    probe.is_empty().then_some(m)
}

/// The message the planner model may read for a turn, by `delivery::plan_for_turn`'s rules: none
/// for a no-tools, read-only BRIEF or fresh-eyes turn, a turn with a carried plan (it wins), a
/// prompt kwant or a bot wrote (no person's message), or a message that only greets, thanks or
/// acknowledges ([`analyzer::needs_no_plan`]). Every other message is asked, keyword match or not:
/// the analyzer is for what the keywords miss ("the export is slow", "Покажи мне макет").
pub fn analyzer_message<'a>(t: &TurnPlanInput<'a>) -> Option<&'a str> {
    if t.no_tools || t.read_only || t.fresh_eyes || t.carried.is_some() {
        return None;
    }
    t.person_message.filter(|m| !analyzer::needs_no_plan(m))
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
pub async fn record_check(db: &crate::db::Database, kwant_id: &str, context_id: &str, wanted: &[Show], hit: bool, missing: &[Show]) {
    let Some(note) = check_note(wanted, hit, missing) else { return };
    let space = match db.get_kwant(kwant_id).await {
        Ok(Some(k)) => SpaceId::new(&k.space_id),
        _ => return,
    };
    if let Err(e) = db.add_delivery_check(&space, kwant_id, context_id, &note).await {
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
/// ("person", the bot's name, or a connected agent's name with "(agent)", text), and the person's
/// own earlier messages for the keyword plan. A message a connected agent or a bot saved as the
/// user's (a hand-off, an outside agent, a bot's chat-API message) is labelled with its sender and
/// is never the person's ([`is_persons`]).
pub fn recent_turns(history: &[Message], prompt: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut msgs: Vec<&Message> = history.iter().filter(|m| m.sender_role == "user" || m.sender_role == "kwant").collect();
    if msgs.last().is_some_and(|m| is_persons(m) && m.text.trim() == prompt.trim()) {
        msgs.pop();
    }
    let label = |m: &Message| match crate::drivers::from_agent(m) {
        Some(agent) => format!("{} (agent)", crate::drivers::frame_safe_name(&agent)),
        None if crate::drivers::from_customer(m).is_some() => format!("{} (customer)", crate::drivers::frame_safe_name(&crate::drivers::from_customer(m).unwrap_or_default())),
        None if is_persons(m) => "person".to_string(),
        None if m.sender_role == "user" => "a bot".to_string(),
        None => m.sender_id.clone(),
    };
    let turns = msgs.iter().filter(|m| !m.text.trim().is_empty()).map(|m| (label(m), m.text.clone())).collect();
    let person = msgs.iter().filter(|m| is_persons(m)).map(|m| m.text.clone()).collect();
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

/// The message [`plan_for_message`] plans for: the person's saved message, never the turn's prompt.
/// The chat's newest message, when it is the person's ([`is_persons`]): not a connected agent's
/// and not one a bot sent through the chat API, so neither ever reaches the planner model.
pub fn planned_message(history: &[Message]) -> Option<&Message> {
    history.last().filter(|m| is_persons(m))
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
) -> Option<(String, DeliveryPlan, Why)> {
    let space = SpaceId::new(&answering.space_id);
    let history = state.db.list_messages(chat_id, RECENT_MESSAGES).await.unwrap_or_default();
    let saved = planned_message(&history)?;
    let (recent, person) = recent_turns(&history, &saved.text);
    let person: Vec<&str> = person.iter().map(String::as_str).collect();
    let message = analyzer_message(&TurnPlanInput { person_message: Some(saved.text.as_str()), recent: &person, ..Default::default() })?;
    let keyword = keyword_plan(message, &person);
    let mut team = vec![Bot { id: answering.id.clone(), name: answering.name.clone(), title: answering.title.clone() }];
    for k in teammates.iter().filter(|k| k.id != answering.id) {
        team.push(Bot { id: k.id.clone(), name: k.name.clone(), title: k.title.clone() });
    }
    let input = Input { message: message.to_string(), recent, team, project: project_of(workspace), notes: notes(state, &space).await };
    let model = planner_model(state).await;
    let (mut plan, why) = analyzer::plan(model.as_deref(), &input, keyword, analyzer::timeout()).await;
    analyzer::align_replier(&mut plan, &answering.id);
    tracing::info!(kwant = %answering.id, kind = ?plan.kind, source = ?plan.source, why = why.as_str(), "delivery plan");
    Some((saved.id.clone(), plan, why))
}

/// The reply the person's newest message reacts to, and the note it earns: the bots' messages
/// since the person's previous message are what they were shown; the reply's plan is the one kept
/// in its meta (else the keyword plan of the message it answered). `None` when there is nothing
/// to judge. `history` is oldest first and ends with the person's new message.
pub fn judge_chat(history: &[Message]) -> Option<(String, Option<String>, Note)> {
    let (now, before) = history.split_last()?;
    // Only the person's own messages react: a connected agent's or a bot's chat-API message never does
    if !is_persons(now) {
        return None;
    }
    let asked_at = before.iter().rposition(is_persons)?;
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

    /// Ada's #470 must-fixes, for the planner too: a routed prompt and a hand-off prompt never
    /// reach the analyzer; only the person's saved message does.
    #[test]
    fn a_routed_prompt_and_a_handoff_prompt_dont_reach_the_analyzer() {
        fn input(m: Option<&str>) -> TurnPlanInput<'_> {
            TurnPlanInput { person_message: m, ..Default::default() }
        }
        let saved = msg("user", "show me the new pricing page", None);
        // Routed: the prompt carries kwant's routing note after the person's words. The turn
        // answers the saved message, and the analyzer reads that, never the note
        let routed = format!("{}\n\n{} Frontend was picked for the words \"pricing page\".)", saved.text, crate::drivers::ROUTING_NOTE);
        let answered = answered_message(Some(&saved), &routed).expect("the routed turn answers the person's message");
        assert_eq!(answered.id, saved.id);
        let t = input(Some(answered.text.as_str()));
        let read = analyzer_message(&t).unwrap();
        assert_eq!(read, "show me the new pricing page");
        assert!(!read.contains(crate::drivers::ROUTING_NOTE));
        // Hand-off: a prompt kwant wrote for a teammate, with the person's words quoted inside it.
        // The chat's newest message is the lead's reply, so the turn answers no message of the person
        let handoff = format!("Growth handed this conversation to you by mentioning you.\n\nThe user's original request:\n{}", saved.text);
        let lead_reply = msg("kwant", "Built it: mockup.png. @Frontend can you check the spacing?", None);
        assert!(answered_message(Some(&lead_reply), &handoff).is_none());
        // ...and even right after the person's message, a prompt that isn't that message is no answer to it
        assert!(answered_message(Some(&saved), &handoff).is_none());
        assert!(analyzer_message(&input(None)).is_none());
        // A connected agent's message isn't the person's
        let agent = msg("user", "show me the new pricing page", Some(serde_json::json!({"from_agent": "Zapier"})));
        assert!(answered_message(Some(&agent), &agent.text).is_none());
    }

    #[test]
    fn the_analyzer_follows_plan_for_turns_rules() {
        let base = || TurnPlanInput { person_message: Some("show me a mockup of the page"), ..Default::default() };
        assert_eq!(analyzer_message(&base()), Some("show me a mockup of the page"));
        // No-tools, read-only BRIEF and fresh-eyes turns: no plan, no planner call
        assert!(analyzer_message(&TurnPlanInput { no_tools: true, ..base() }).is_none());
        assert!(analyzer_message(&TurnPlanInput { read_only: true, ..base() }).is_none());
        assert!(analyzer_message(&TurnPlanInput { fresh_eyes: true, ..base() }).is_none());
        // A carried plan (the decided reply's) wins
        assert!(analyzer_message(&TurnPlanInput { carried: Some(keyword_plan("explain it", &[])), ..base() }).is_none());
        // Thanks and acknowledgements ask for nothing: no plan
        for ack in ["thanks!", "lgtm", "I see, thanks", "awesome, thanks"] {
            assert!(analyzer_message(&TurnPlanInput { person_message: Some(ack), ..base() }).is_none(), "{ack}");
        }
        // The job's plan reaches only the turn answering the message it was made from
        let job = format!("job-{}", uuid::Uuid::new_v4());
        remember(&job, "m1", keyword_plan("show me mockup image", &[]));
        assert!(plan_for_answer(&job, None).is_none(), "a hand-off turn of the same job");
        forget(&job);
    }

    /// Ada's #474 M1: the live gate is the 14:41 rule. Every message but greetings, thanks or
    /// acknowledgements alone reaches the analyzer, keyword match or not.
    #[test]
    fn every_message_but_greetings_and_thanks_is_asked_live() {
        let input = |m| TurnPlanInput { person_message: Some(m), ..Default::default() };
        for m in ["the export is slow", "Покажи мне макет", "the footer overlaps on mobile"] {
            assert_eq!(analyzer_message(&input(m)), Some(m), "{m}");
        }
        for m in ["thanks!", "ok 👍", "hi"] {
            assert!(analyzer_message(&input(m)).is_none(), "{m}");
        }
    }

    /// Ada's #474 S2: a connected agent's message saved as the user's is labelled with its sender
    /// and is never one of the person's own earlier messages.
    #[test]
    fn an_agents_message_is_not_the_persons() {
        let agent = msg("user", "Fix differences on every page!", Some(serde_json::json!({"from_agent": "Zap\nier [bot]"})));
        let chat = vec![msg("user", "what is this??", None), agent, msg("kwant", "A bar.", None), msg("user", "show me", None)];
        let (turns, person) = recent_turns(&chat, "show me");
        assert_eq!(turns[1], ("Zap ier _bot_ (agent)".to_string(), "Fix differences on every page!".to_string()));
        assert_eq!(turns[0].0, "person");
        assert_eq!(person, vec!["what is this??".to_string()]);
    }

    /// #469's one rule for "the person" ([`is_persons`]): a message a bot sent through the chat
    /// API (`meta.sent_by_bot`) is saved as the user's, but never reaches the planner model, is
    /// never what a turn answers, never one of the person's earlier messages and never a reaction.
    #[test]
    fn a_bots_chat_api_message_gets_no_planner_call() {
        let by_bot = msg("user", "Show me the new pricing page mockup", Some(serde_json::json!({"sent_by_bot": true})));
        let theirs = msg("user", "Show me the new pricing page mockup", None);
        assert!(planned_message(std::slice::from_ref(&by_bot)).is_none());
        assert_eq!(planned_message(std::slice::from_ref(&theirs)).map(|m| m.id.as_str()), Some(theirs.id.as_str()));
        assert!(answered_message(Some(&by_bot), &by_bot.text).is_none());
        assert!(answered_message(Some(&theirs), &theirs.text).is_some());
        let reply = msg("kwant", "Here is the pricing page.", None);
        let (turns, person) = recent_turns(&[by_bot.clone(), reply.clone(), theirs.clone()], &theirs.text);
        assert_eq!(turns[0].0, "a bot");
        assert!(person.is_empty(), "{person:?}");
        // A bot's message after a reply is no reaction to it
        let again = msg("user", "show me", Some(serde_json::json!({"sent_by_bot": true})));
        assert!(judge_chat(&[theirs.clone(), reply.clone(), again]).is_none());
        assert!(judge_chat(&[theirs, reply, msg("user", "show me", None)]).is_some());
    }

    #[test]
    fn a_jobs_plan_is_kept_for_its_turns_and_dropped_after() {
        let job = format!("job-{}", uuid::Uuid::new_v4());
        assert!(plan_for_answer(&job, Some("m1")).is_none());
        remember(&job, "m1", keyword_plan("show me mockup image", &[]));
        assert_eq!(plan_for_answer(&job, Some("m1")).unwrap().kind, Deliverable::Visual);
        // Only the turn that answers the message it was made from: a hand-off turn answers none
        assert!(plan_for_answer(&job, None).is_none());
        assert!(plan_for_answer(&job, Some("m2")).is_none());
        forget(&job);
        assert!(plan_for_answer(&job, Some("m1")).is_none());
    }
}
