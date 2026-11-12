//! Keep going (0.9.2): a lead's chain never just stops. After a lead's results turn (see
//! [`super::delegation`]), its hand-offs and the list of open asks, kwnt checks whether the chain
//! is still moving: something handed on or still open, a question waiting on the person, a
//! pause, the person writing or pressing Stop, or the lead ending with `Done: …`. When none of
//! that holds the chain has stalled (Oleks's Studio/Director video, 2026-11-11: "Next decision
//! for you: review animatic v10 … Nothing else is waiting on you", then nothing), and:
//!
//! - a reply whose last line asks the person something becomes a question card at once;
//! - else the lead gets a keep-going turn in the same chat, at most [`DEFAULT_TURNS`] per person
//!   message (counted like the hand-off ceiling, see [`super::routes`]'s request start);
//! - past that, one "Keep going?" card. "Keep going" is a person message, so it brings a fresh
//!   allowance and starts a keep-going turn; "That's enough for now" ends it.
//!
//! The `Done:` line is stored on the reply's meta as [`DONE_KEY`]. A lead whose last line is
//! `Blocked: <what is blocked>. <what would unblock it>` (0.9.4) never gets a keep-going turn: the
//! line becomes a question card at once, like a question, and its answer resumes the chain. The
//! save step stores any reply's `Blocked:` line on its meta as [`BLOCKED_KEY`].

use super::routes::{try_kwant_turn, TurnOpts, TurnScope};
use super::AppState;
use crate::db::models::{Kwant, Message};
use crate::db::questions::{Question, QuestionPlace, QuestionReason};
use crate::db::waiting::NewWaitingItem;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Keep-going turns per person message; `0` turns them off. A plain setting (no API), for a
/// kill switch and for tests that predate keep-going turns.
pub const SETTING: &str = "keep_going_turns";
pub const DEFAULT_TURNS: usize = 2;
const MAX_TURNS: usize = 5;
/// On a keep-going turn's reply (or its failure note): which turn of the allowance it was.
pub const TURN_KEY: &str = "keep_going";
/// On the "Keep going?" card's message.
pub const CARD_KEY: &str = "keep_going_card";
/// On a card kwant made from a lead's reply that asked the person in prose: its answer resumes
/// the lead's chain (hand-offs, then this check again).
pub const ASK_KEY: &str = "keep_going_ask";
/// On a reply that ends with `Done: …`: `{text}`, what the lead said it checked.
pub const DONE_KEY: &str = "chain_done";
/// On a reply that ends with `Blocked: …` (0.9.4): `{text}`, what is blocked and what unblocks it.
pub const BLOCKED_KEY: &str = "blocked";
pub const KEEP_GOING: &str = "Keep going";
pub const ENOUGH: &str = "That's enough for now";
/// What a keep-going turn is told.
pub const PROMPT: &str = "The person's request isn't marked done, nothing is handed on and nothing is asked. Do the next \
    step now: hand it on or do it yourself. If it really is finished, check it and end with `Done: …`. If only the \
    person can unblock it, ask with the question tool.";
/// The person messages a keep-going turn quotes.
const QUOTED: usize = 3;
const QUOTE_CHARS: usize = 600;
const CARD_LINE_CHARS: usize = 160;

/// How many keep-going turns one person message allows.
pub async fn turns_allowed(state: &AppState) -> usize {
    state
        .db
        .get_setting(SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.min(MAX_TURNS))
        .unwrap_or(DEFAULT_TURNS)
}

/// The `Done: …` line a reply ends with, if it does: its last non-empty line, markdown
/// decoration aside, starts with "Done:" and says what was done.
pub fn done_line(reply: &str) -> Option<String> {
    end_line(reply, "done:")
}

/// The `Blocked: <what is blocked>. <what would unblock it>` line a reply ends with, if it does
/// (0.9.4): parsed like [`done_line`].
pub fn blocked_line(reply: &str) -> Option<String> {
    end_line(reply, "blocked:")
}

/// The reply's last non-empty line, markdown decoration aside, when it starts with `head`
/// (lowercase, ASCII, ending in ':'): what follows it.
fn end_line(reply: &str, head: &str) -> Option<String> {
    let last = reply.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let bare = last.trim_start_matches(['*', '_', '-', '>', '`', ' ']);
    if !bare.get(..head.len())?.eq_ignore_ascii_case(head) {
        return None;
    }
    let what = bare[head.len()..].trim().trim_matches(['*', '_', '`', ' ']).trim();
    (!what.is_empty()).then(|| what.to_string())
}

/// What the chat looked like before a lead turn: whatever isn't here afterwards is new.
pub(super) struct Snapshot {
    context: String,
    messages: HashSet<String>,
    delegations: HashSet<String>,
    last_user: Option<String>,
}

/// The chat `context` and `lead`'s hand-offs from it, before a turn of `lead` there.
pub(super) async fn snapshot(state: &AppState, context: &str, lead: &Kwant) -> Snapshot {
    let msgs = state.db.list_messages(context, 100).await.unwrap_or_default();
    Snapshot {
        context: context.to_string(),
        messages: msgs.iter().map(|m| m.id.clone()).collect(),
        delegations: lead_delegations(state, context, lead).await.into_iter().map(|d| d.id).collect(),
        last_user: msgs.iter().rev().find(|m| m.sender_role == "user").map(|m| m.id.clone()),
    }
}

async fn lead_delegations(state: &AppState, context: &str, lead: &Kwant) -> Vec<crate::db::threads::Delegation> {
    state
        .db
        .list_delegations(Some(&lead.id), None, 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.origin_context().unwrap_or(&lead.id) == context)
        .collect()
}

fn meta_of(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// Where this person message's request starts: after the person's last message, or after a
/// hand-off ceiling notice they pressed Continue on (like the hand-off ceiling's count).
fn request_tail(msgs: &[Message]) -> &[Message] {
    let start = msgs.iter().rposition(|m| {
        let meta = meta_of(m);
        m.sender_role == "user" || (meta["kind"] == super::budget::CAPPED_KIND && meta["resumed"] == true)
    });
    match start {
        Some(i) => &msgs[i + 1..],
        None => msgs,
    }
}

#[derive(Debug, PartialEq)]
enum Verdict {
    /// Something still moves the chain, or the person or a pause holds it.
    Moving,
    /// The lead ended with `Done: …`.
    Done(String),
    /// The lead's last line asks the person this, in prose.
    Asks(String),
    Stalled,
}

async fn verdict(scope: &TurnScope, lead: &Kwant, snap: &Snapshot, reply: &str) -> Verdict {
    let state = &scope.state;
    // The person pressed Stop
    if scope.cancel_token.is_cancelled() {
        return Verdict::Moving;
    }
    let msgs = state.db.list_messages(&snap.context, 100).await.unwrap_or_default();
    // The person wrote: their message has its own turn
    if msgs.iter().rev().find(|m| m.sender_role == "user").map(|m| &m.id) != snap.last_user.as_ref() {
        return Verdict::Moving;
    }
    for m in msgs.iter().filter(|m| !snap.messages.contains(&m.id)) {
        let meta = meta_of(m);
        let kind = meta["kind"].as_str().unwrap_or("");
        // A pause the person lifts (the chain's budget, the hand-off ceiling, a bot's budget),
        // one list of what the team needs from them, or another bot speaking in the chat
        if [super::budget::KIND, super::budget::CAPPED_KIND, super::bot_budget::STOPPED_KIND].contains(&kind)
            || meta.get(super::asks::META_KEY).is_some()
            || (m.sender_role == "kwant" && m.sender_id != lead.name && m.sender_id != lead.id)
        {
            return Verdict::Moving;
        }
    }
    let delegations = lead_delegations(state, &snap.context, lead).await;
    // Handed on, or a hand-off of this lead here is still open or parked
    if delegations.iter().any(|d| !snap.delegations.contains(&d.id) || d.is_open()) {
        return Verdict::Moving;
    }
    // A question card waits on the person here, or in a thread of this request's hand-offs
    if !state.db.list_questions(Some("open"), Some(&snap.context), None).await.unwrap_or_default().is_empty() {
        return Verdict::Moving;
    }
    let since = request_tail(&msgs).first().and_then(|m| m.created_at);
    for d in delegations.iter().filter(|d| since.is_none_or(|s| d.created_at >= s)) {
        if !state.db.list_questions(Some("open"), Some(&d.thread_id), None).await.unwrap_or_default().is_empty() {
            return Verdict::Moving;
        }
    }
    // Monthly bot budget or the space's spend stop: nothing runs until the person lifts it
    if super::bot_budget::held(state, &lead.id).await.is_some() || super::spend_stop::held(state, &lead.space_id).await.is_some() {
        return Verdict::Moving;
    }
    if let Some(done) = done_line(reply) {
        return Verdict::Done(done);
    }
    // Blocked: only the person (or a setting) unblocks it, so it is asked, never pushed on
    if let Some(blocked) = blocked_line(reply) {
        return Verdict::Asks(format!("Blocked: {blocked}"));
    }
    let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
    if let Some(q) = crate::router::sticky::person_question(reply).filter(|_| !super::asks::hands_on(reply, &names)) {
        return Verdict::Asks(q);
    }
    Verdict::Stalled
}

/// After a lead's turn in a person's chat (its results turn, a keep-going turn, its turn on a
/// late answer) once its hand-offs ran: keeps the chain going if it stalled. `snap` is the chat
/// before that turn, `reply` the turn's reply.
pub(super) async fn after_lead_turn(scope: &TurnScope, lead: &Kwant, mut snap: Snapshot, mut reply: String) {
    // Only where the lead answers the person: a lead inside a hand-off reports to its own lead
    if scope.delegation_depth != 0 {
        return;
    }
    let state = &scope.state;
    loop {
        match verdict(scope, lead, &snap, &reply).await {
            Verdict::Moving => return,
            Verdict::Done(text) => return mark_done(state, &snap, lead, &text).await,
            Verdict::Asks(q) => {
                ask_card(scope, lead, &q, &[], ASK_KEY).await;
                return;
            }
            Verdict::Stalled => {}
        }
        let allowed = turns_allowed(state).await;
        if allowed == 0 {
            return;
        }
        let msgs = state.db.list_messages(&snap.context, 100).await.unwrap_or_default();
        let tail = request_tail(&msgs);
        let used = tail.iter().filter(|m| meta_of(m).get(TURN_KEY).is_some()).count();
        if used >= allowed {
            // One card per person message, never a second
            if !tail.iter().any(|m| meta_of(m).get(CARD_KEY).is_some()) {
                tracing::info!(lead = %lead.id, context = %snap.context, "a lead stopped before its request was done; asking the person");
                let question = format!("{} stopped before the job was done: {}. Keep going?", lead.name, first_line(&reply));
                ask_card(scope, lead, &question, &[KEEP_GOING.into(), ENOUGH.into()], CARD_KEY).await;
            }
            return;
        }
        tracing::info!(lead = %lead.id, context = %snap.context, turn = used + 1, "a lead's chain stalled; a keep-going turn");
        snap = snapshot(state, &snap.context, lead).await;
        match keep_going_turn(scope, lead, used + 1).await {
            Some(r) => reply = r,
            None => return,
        }
    }
}

/// One keep-going turn of `lead` in the chat, then its hand-offs. `None` when it failed.
async fn keep_going_turn(scope: &TurnScope, lead: &Kwant, n: usize) -> Option<String> {
    let state = &scope.state;
    let context = scope.context_id(lead);
    let msgs = state.db.list_messages(&context, 100).await.unwrap_or_default();
    let mut asked: Vec<&Message> = msgs.iter().rev().filter(|m| m.sender_role == "user").take(QUOTED).collect();
    asked.reverse();
    let quoted: Vec<String> = asked.iter().map(|m| format!("> {}", clip(&m.text, QUOTE_CHARS).replace('\n', "\n> "))).collect();
    let prompt = format!(
        "[kwnt: {PROMPT}]\n\nThe person's last messages here, oldest first:\n{}\n\n{}",
        if quoted.is_empty() { "(none)".to_string() } else { quoted.join("\n\n") },
        super::delegation::handoff_rules(),
    );
    let mut opts = TurnOpts::default();
    opts.label = Some("Keeping the work going".into());
    opts.extra_meta = Some(json!({ TURN_KEY: n }));
    opts.failure_meta = Some(json!({ TURN_KEY: n }));
    opts.purpose = Some("keep_going".into());
    let reply = try_kwant_turn(scope, lead, prompt, opts).await.ok()?;
    let user_prompt = asked.last().map(|m| m.text.clone()).unwrap_or_default();
    super::routes::run_handoffs_boxed(scope, std::slice::from_ref(&lead.id), &[(lead.clone(), reply.clone())], &user_prompt).await;
    super::asks::consolidate(scope).await;
    Some(reply)
}

/// The first line of a reply, for the card: plain, short, without a closing full stop.
fn first_line(reply: &str) -> String {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it gave no reply");
    let line = line.trim_start_matches(['#', '*', '_', '>', '-', ' ']).trim_end_matches(['*', '_', ' ']);
    clip(line, CARD_LINE_CHARS).trim_end_matches(['.', ':', ' ']).to_string()
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Stores the lead's `Done: …` on its newest reply here since `snap`.
async fn mark_done(state: &AppState, snap: &Snapshot, lead: &Kwant, text: &str) {
    let msgs = state.db.list_messages(&snap.context, 100).await.unwrap_or_default();
    let Some(m) = msgs.iter().rev().find(|m| !snap.messages.contains(&m.id) && super::routes::is_reply_by(m, &lead.name)) else {
        return;
    };
    super::asks::set_meta_key(state, m, DONE_KEY, json!({ "text": text, "at": Utc::now() })).await;
}

/// A question card from `lead` in the scope's chat with no turn waiting on it; its answer comes
/// back as a new turn (see [`super::questions`]). It replaces the plain "waiting on a reply"
/// entry the reply left, so the person gets one reminder, for the card.
async fn ask_card(scope: &TurnScope, lead: &Kwant, question: &str, choices: &[String], key: &str) -> Option<Question> {
    let state = &scope.state;
    let direct = match (&scope.channel_id, &scope.thread_id) {
        (None, None) => Some(scope.kwant_id.clone().unwrap_or_else(|| lead.id.clone())),
        _ => None,
    };
    let place = QuestionPlace {
        kwant_id: lead.id.clone(),
        channel_id: scope.channel_id.clone(),
        direct_kwant_id: direct.clone(),
        thread_id: scope.thread_id.clone(),
        turn_id: None,
    };
    let question: String = question.chars().take(crate::db::questions::MAX_QUESTION_CHARS).collect();
    let q = super::questions::ask_without_waiter(state, &place, &lead.name, &question, choices, QuestionReason::default()).await?;
    if let Some(m) = card_message(state, &q).await {
        super::asks::set_meta_key(state, &m, key, json!(true)).await;
    }
    let _ = state.db.close_plain_waiting(&lead.id, scope.channel_id.as_deref(), direct.as_deref(), scope.thread_id.as_deref()).await;
    let hours = super::waiting::follow_up_hours(state).await;
    let item = NewWaitingItem {
        kwant_id: lead.id.clone(),
        channel_id: q.channel_id.clone(),
        direct_kwant_id: q.direct_kwant_id.clone(),
        thread_id: q.thread_id.clone(),
        question_id: Some(q.id.clone()),
        message_id: q.message_id.clone(),
        question: q.question.clone(),
        due_at: (hours > 0).then(|| Utc::now() + Duration::hours(hours)),
    };
    if matches!(state.db.add_waiting(&item).await, Ok(Some(_))) {
        state.scheduler_wake.notify_one();
    }
    Some(q)
}

/// The person answered a question kwant asked with no turn waiting. For the "Keep going?" card:
/// "That's enough for now" ends the chain, anything else starts a keep-going turn (the answer is
/// already saved as their message, so it has a fresh allowance). `false` for any other card.
pub(super) async fn on_card_answer(state: &AppState, q: &Question, answer: &str) -> bool {
    if !card_has(state, q, CARD_KEY).await {
        return false;
    }
    if answer.trim().eq_ignore_ascii_case(ENOUGH) {
        return true;
    }
    let Ok(Some(lead)) = state.db.get_kwant(&q.kwant_id).await else {
        return true;
    };
    let job = state.turns.start_job([q.context_id(), q.id.clone()]);
    let Ok(scope) = super::routes::scope_for(state, &lead, q.channel_id.clone(), q.thread_id.clone(), &job, 0).await else {
        return true;
    };
    tokio::spawn(async move {
        let _job = job;
        let context = scope.context_id(&lead);
        let snap = snapshot(&scope.state, &context, &lead).await;
        // The answer is the person's message: a fresh allowance, of which this is the first turn
        if let Some(reply) = keep_going_turn(&scope, &lead, 1).await {
            after_lead_turn(&scope, &lead, snap, reply).await;
        }
    });
    true
}

async fn card_message(state: &AppState, q: &Question) -> Option<Message> {
    state.db.get_message(q.message_id.as_deref()?).await.ok().flatten()
}

/// Whether `q`'s card message carries `key` (see [`CARD_KEY`], [`ASK_KEY`]).
pub(super) async fn card_has(state: &AppState, q: &Question, key: &str) -> bool {
    card_message(state, q).await.is_some_and(|m| meta_of(&m).get(key).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_done_line_ends_the_reply() {
        assert_eq!(done_line("Summary.\n\nDone: checked the cover and the PDF.").as_deref(), Some("checked the cover and the PDF."));
        assert_eq!(done_line("**Done:** all 3 drafts reviewed\n").as_deref(), Some("all 3 drafts reviewed"));
        assert_eq!(done_line("- done: tests pass").as_deref(), Some("tests pass"));
        assert_eq!(done_line("`Done: built`").as_deref(), Some("built"));
        // Not the last line, empty, or only a word that starts with "done"
        assert_eq!(done_line("Done: checked it.\nNext I'll draft chapter 2."), None);
        assert_eq!(done_line("Done:"), None);
        assert_eq!(done_line("Doneness: fine"), None);
        assert_eq!(done_line("All done."), None);
        assert_eq!(done_line(""), None);
        assert_eq!(done_line("Dön"), None);
    }

    #[test]
    fn a_blocked_line_ends_the_reply() {
        let real = "The render failed twice.\n\nBlocked: terminal commands are off for this bot on the shared Mac. Allow Terminal for this team on that computer, then say continue.";
        assert_eq!(
            blocked_line(real).as_deref(),
            Some("terminal commands are off for this bot on the shared Mac. Allow Terminal for this team on that computer, then say continue.")
        );
        assert_eq!(blocked_line("**Blocked:** no key for the Stripe API. Add it in Settings.").as_deref(), Some("no key for the Stripe API. Add it in Settings."));
        assert_eq!(blocked_line("- blocked: CI is down\n\n").as_deref(), Some("CI is down"));
        // Not the last line, empty, a Done line, or "blocked" inside a sentence
        assert_eq!(blocked_line("Blocked: CI.\nI'll retry later."), None);
        assert_eq!(blocked_line("Blocked:"), None);
        assert_eq!(blocked_line("Done: checked it."), None);
        assert_eq!(blocked_line("The deploy is blocked: no key."), None);
        assert_eq!(done_line("Blocked: CI is down."), None);
        assert_eq!(blocked_line(""), None);
    }

    #[test]
    fn the_card_quotes_a_plain_first_line() {
        assert_eq!(first_line("\n## Passed Builder's floor-dot task.\nMore."), "Passed Builder's floor-dot task");
        assert_eq!(first_line(""), "it gave no reply");
        assert!(first_line(&"x".repeat(400)).chars().count() <= CARD_LINE_CHARS + 1);
    }

    #[test]
    fn the_prompt_names_the_three_ends() {
        assert!(PROMPT.contains("hand it on or do it yourself"));
        assert!(PROMPT.contains("`Done: …`"));
        assert!(PROMPT.contains("question tool"));
    }
}
