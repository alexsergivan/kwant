//! Decide first: a bot's final reply that ends by asking the person to pick between reversible
//! options is held back. The bot gets one hidden round to finish the job instead: it runs the
//! checks its reply lists as not done, hears up to two teammates, decides, applies what it decided
//! and posts one reply that says what it chose, why, and how to change it. Each choice is kept in
//! the reply's `meta.decisions` with a Change button (see [`change`]), and every pick the person
//! makes or changes is kept as a decision note (see [`crate::db::decisions`]), which the bots read
//! back as "How the person usually decides" to predict the next one.
//!
//! The gate never holds a reply that asks about something that can't be undone or reaches outside
//! (sending, paying, deleting, contacting someone, passwords and sign-ins), a reply after the
//! person said they want to pick themselves, a reply that hands on to a teammate, a delegation or
//! system turn, or a second reply in the same chain (one round per person message). When the chain's budget can't fit the extra round, or
//! the round fails or is stopped, the original reply is posted as it was.

use super::routes::{turn_once, TurnOpts, TurnScope};
use super::AppState;
use crate::db::decisions::{DecisionNote, NewDecisionNote};
use crate::db::models::{Kwant, Message, NewMessage};
use crate::db::spaces::SpaceId;
use crate::drivers::RuntimeEvent;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// Setting key: `"false"` turns the gate (and the `ask_user` reason check) off. On by default.
pub const SETTING: &str = "decide_first";
/// Fresh tokens the hidden round needs to be left in the chain's budget.
pub const RESERVE_TOKENS: u64 = 30_000;
/// Teammates asked for their view at most: the one most relevant to the open choices, or two when
/// the reply's choices or undone checks name both (see [`consultees`]).
pub const MAX_CONSULTS: usize = 2;
/// The `purpose` of the hidden round's turns in `GET /api/turns` (`turns[]`).
pub const PURPOSE: &str = "decide";
/// Non-empty lines at a reply's end looked at for open choices.
const TAIL_LINES: usize = 12;

/// Setting key: `"false"` turns work-ahead runs in quiet chats off. On by default.
pub const WORK_AHEAD_SETTING: &str = "work_ahead";
/// Setting key: work-ahead runs per space per day.
pub const WORK_AHEAD_CAP_SETTING: &str = "work_ahead_daily_cap";
pub const DEFAULT_WORK_AHEAD_CAP: u64 = 3;
pub const MAX_WORK_AHEAD_CAP: u64 = 10;

fn is_off(v: Option<String>) -> bool {
    matches!(v.as_deref().map(str::trim), Some("false" | "0" | "off"))
}

/// Whether work-ahead runs are on (the default).
pub async fn work_ahead_enabled(state: &AppState) -> bool {
    !is_off(state.db.get_setting(WORK_AHEAD_SETTING).await.ok().flatten())
}

/// Work-ahead runs per space per day; a missing or bad value means the default.
pub async fn work_ahead_cap(state: &AppState) -> u64 {
    state
        .db
        .get_setting(WORK_AHEAD_CAP_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| (1..=MAX_WORK_AHEAD_CAP).contains(n))
        .unwrap_or(DEFAULT_WORK_AHEAD_CAP)
}

/// Whether decide-first is on (the default).
pub async fn enabled(state: &AppState) -> bool {
    !is_off(state.db.get_setting(SETTING).await.ok().flatten())
}

/// A reply the gate held back, with what was about to be saved.
pub struct Held {
    pub turn_id: String,
    /// What the gate found in it.
    pub gate: Gate,
    pub text: String,
    pub msg: NewMessage,
    pub meta: Option<Value>,
}

/// Shared with a turn's save step (see [`TurnOpts`]): whether to hold a reply with open choices,
/// what was held, and the message that was saved.
#[derive(Default)]
pub struct Hook {
    pub hold_choices: bool,
    /// Names whose @mention on the reply's last line hands it on (then it isn't held).
    pub teammates: Vec<String>,
    /// The person's message asked for a step they have to own (see [`person_asked_hard`]).
    pub person_hard: bool,
    pub held: Option<Held>,
    pub saved: Option<Message>,
}

pub type HookSlot = Arc<Mutex<Hook>>;

// The heuristic

/// What the gate found in a reply.
#[derive(Debug, Default, PartialEq)]
pub struct Gate {
    /// Lines that ask the person to choose.
    pub choices: Vec<String>,
    /// Lines that say something wasn't checked.
    pub unchecked: Vec<String>,
    /// Lines that ask about a step the person has to own; they stay for the person.
    pub kept: Vec<String>,
}

const CHOICE_PHRASES: &[&str] = &[
    "decisions for you",
    "decision for you",
    "your call",
    "up to you",
    "which do you want",
    "which would you like",
    "which one do you",
    "which one should",
    "do you prefer",
    "would you prefer",
    "should i go with",
    "should we go with",
    "let me know which",
    "tell me which",
    "pick one",
    "when you pick",
    "once you pick",
    "once you choose",
    "until you answer",
    "until you decide",
    "choose between",
    "which option",
    "which direction",
    "which version",
    "which approach",
];

/// Words that make a question a choice.
const CHOICE_WORDS: &[&str] = &[
    "prefer", "go with", "pick", "choose", "option", "direction", "version", "approach", "should i ", "should we ", "do you want",
    "would you like", "want me to", "like me to", "shall i ",
];

/// Soft offers that hand a reversible choice back without a question mark ("say if you want me to
/// add a subject", "tell me the path and I'll move it", "if you'd rather, …"): a lead-in…
const OFFER_LEADS: &[&str] = &["say if", "say so", "say the word", "just say", "tell me", "let me know", "give me a shout"];
/// …with what it offers.
const OFFER_TAILS: &[&str] = &[
    "i'll", "i will", "i can", "want me", "you want", "you'd like", "you would like", "you'd rather", "you would rather",
    "you prefer", "you'd prefer", "the path", "which",
];
/// Offers that need no lead-in.
const OFFER_PHRASES: &[&str] = &["if you'd rather", "if you would rather", "if you'd prefer", "if you would prefer", "if you prefer"];
/// Catch-alls that only close a reply ("let me know if you need anything else").
const GENERIC: &[&str] = &[
    "anything else", "anything more", "anything further", "if anything", "anything comes up", "any questions", "any other questions",
    "have questions", "need anything", "need more help", "need any help", "need further help", "help further",
];

const UNCHECKED: &[&str] = &[
    "not checked",
    "unchecked",
    "nobody has",
    "nobody looked",
    "no one has",
    "no one looked",
    "only looked at",
    "haven't checked",
    "have not checked",
    "haven't looked",
    "still need checking",
    "still needs checking",
    "not verified",
    "unverified",
    "untested",
    "not tested",
    "nobody ran",
    "no one ran",
];

/// Whole words that mark a step the person has to own: it can't be undone or reaches outside.
const HARD_WORDS: &[&str] = &[
    "send", "sending", "sent", "signup", "email", "emails", "emailing", "e-mail", "post", "posting", "publish", "publishing", "tweet",
    "pay", "paying", "payment", "buy", "buying", "purchase", "subscribe", "subscription", "charge", "refund", "delete",
    "deleting", "wipe", "password", "passwords", "credential", "credentials", "token", "contact", "contacting", "invite",
    "transfer", "irreversible", "permanent", "permanently", "bitwarden", "login",
];

/// Phrases with the same meaning as [`HARD_WORDS`].
const HARD_PHRASES: &[&str] = &[
    "sign in", "sign-in", "log in", "sign up", "sign us up", "sign me up", "checkout", "api key", "can't be undone", "cannot be undone",
    "can't undo", "go live", "reach out", "message them",
];

/// When the person asked for a step they have to own (send this, sign us up), a choice line with
/// one of these is about that step: which plan, whether to go ahead, when.
const HARD_REFS: &[&str] = &[
    "plan", "tier", "confirm", "go ahead", "proceed", "account", "annual", "monthly", "yearly", "per month", "per year", "price", "now",
];

/// The person said they want to pick themselves.
const WANTS_TO_PICK: &[&str] = &[
    "let me pick",
    "let me choose",
    "let me decide",
    "i'll pick",
    "i'll choose",
    "i'll decide",
    "i will pick",
    "i will choose",
    "i will decide",
    "ask me",
    "give me options",
    "give me the options",
    "show me options",
    "show me the options",
    "i want to choose",
    "i want to pick",
    "i want to decide",
    "don't decide",
    "do not decide",
    "i decide",
    "my call",
];

fn words(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_matches(|c| c == '\'' || c == '-').to_string())
        .collect()
}

/// Whether `line` (lower case) asks about a step the person has to own. Asking the person to send
/// something to the bot ("send me their VAT ID") isn't one.
fn hard_line(line: &str) -> bool {
    let line = line.replace('’', "'");
    let ws = words(&line);
    let to_us = |i: usize| matches!(ws.get(i + 1).map(String::as_str), Some("me" | "us"));
    let to_person = |w: &str| w.starts_with("send") || w == "sent" || w.starts_with("email") || w == "e-mail";
    ws.iter().enumerate().any(|(i, w)| HARD_WORDS.contains(&w.as_str()) && !(to_person(w) && to_us(i)))
        || HARD_PHRASES.iter().any(|p| line.contains(p))
}

/// Whether `line` (lower case) hands a reversible choice back softly (see [`OFFER_LEADS`]).
fn soft_offer(line: &str) -> bool {
    let line = line.replace('’', "'");
    if GENERIC.iter().any(|g| line.contains(g)) {
        return false;
    }
    OFFER_PHRASES.iter().any(|p| line.contains(p))
        || OFFER_LEADS.iter().any(|l| line.match_indices(l).any(|(i, _)| OFFER_TAILS.iter().any(|t| line[i..].contains(t))))
}

/// The sentences of `line` (lower case) that ask or offer something.
fn asking_sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    for (i, c) in line.char_indices() {
        let end = i + 1 == line.len() || bytes.get(i + 1).is_some_and(|n| n.is_ascii_whitespace());
        if matches!(c, '.' | '?' | '!') && end {
            let s = &line[start..=i];
            if s.contains('?') || soft_offer(s) || CHOICE_PHRASES.iter().any(|p| s.contains(p)) {
                out.push(s);
            }
            start = i + 1;
        }
    }
    let rest = &line[start.min(line.len())..];
    if !rest.trim().is_empty() {
        out.push(rest);
    }
    out
}

/// Whether `line` (lower case) names options like "A, B or C" or "A or B?".
fn names_options(line: &str) -> bool {
    let ws = words(line);
    ws.windows(3).any(|w| w[1] == "or" && w[0].len() == 1 && w[2].len() == 1 && w[0] != "i" && w[2] != "i")
        || ws.windows(2).any(|w| (w[0] == "option" || w[0] == "options") && w[1].len() == 1)
}

/// A line that is a question in the reply's tail: ends in "?", or has one before trailing markup.
fn is_question(line: &str) -> bool {
    line.trim_end_matches(|c: char| c == '*' || c == '_' || c == ')' || c.is_whitespace()).ends_with('?')
}

/// What the gate knows besides the reply.
#[derive(Debug, Default, Clone, Copy)]
pub struct GateCtx {
    /// The person's message asks for a step they have to own (see [`person_asked_hard`]).
    pub person_hard: bool,
    /// The reply already carries choices the bot made (`meta.decisions`).
    pub decided: bool,
}

/// Whether a reply's last line hands on to a teammate: it speaks to them ("@Pia, can you…",
/// "@Pia: …", or starts with "@Pia"). "I can have @Pia check it. Do you want that?" speaks to the
/// person, so it doesn't.
pub fn hands_on(text: &str, names: &[&str]) -> bool {
    let last = text.trim_end().lines().last().unwrap_or("").to_lowercase();
    let line = last.trim_start_matches(|c: char| c.is_whitespace() || "-*•>_".contains(c));
    names.iter().any(|n| {
        let at = format!("@{}", n.to_lowercase());
        line.starts_with(&at) || [",", ":", " please", " can you", " could you"].iter().any(|t| line.contains(&format!("{at}{t}")))
    })
}

/// Whether the person's message asks for a step they have to own (send, pay, sign up, delete…).
pub fn person_asked_hard(person_text: &str) -> bool {
    hard_line(&person_text.to_lowercase())
}

/// [`gate_in`] without context.
pub fn gate(text: &str) -> Option<Gate> {
    gate_in(text, GateCtx::default())
}

/// The reply's open choices for the person, if it ends with any: a choice phrase ("decisions for
/// you", "which do you want"), options like "A, B or C?", a question with a choice word or " or ",
/// or a soft offer ("say if you want me to…", "tell me the path and I'll…"). A choice line is kept
/// for the person when it, or the line before it in its paragraph, is about a step the person has
/// to own (see [`HARD_WORDS`], and [`HARD_REFS`] when the person asked for one). `None` when no
/// reversible choice is left, or when the reply already made its choices (`ctx.decided`) and asks
/// no question.
pub fn gate_in(text: &str, ctx: GateCtx) -> Option<Gate> {
    // Non-empty lines, each with whether a blank line came before it
    let mut lines: Vec<(&str, bool)> = Vec::new();
    let mut gap = true;
    for raw in text.lines().map(str::trim) {
        if raw.is_empty() {
            gap = true;
            continue;
        }
        lines.push((raw, gap));
        gap = false;
    }
    let from = lines.len().saturating_sub(TAIL_LINES);
    let mut out = Gate::default();
    let mut asks = false;
    for i in from..lines.len() {
        let (line, starts_paragraph) = lines[i];
        let l = line.to_lowercase();
        let question = is_question(&l);
        let phrase = CHOICE_PHRASES.iter().any(|p| l.contains(p));
        let choice = phrase
            || soft_offer(&l)
            || (question
                && (names_options(&l) || l.contains(" or ") || CHOICE_WORDS.iter().any(|w| l.contains(w)))
                && !crate::router::sticky::is_closer(&l));
        if choice {
            let before = (!starts_paragraph && i > 0).then(|| lines[i - 1].0.to_lowercase());
            let refs_hard = |s: &str| {
                let ws = words(s);
                HARD_REFS.iter().any(|r| if r.contains(' ') { s.contains(r) } else { ws.iter().any(|w| w == r) })
            };
            let about_request = ctx.person_hard && asking_sentences(&l).iter().any(|s| refs_hard(s));
            if hard_line(&l) || before.as_deref().is_some_and(hard_line) || about_request {
                out.kept.push(line.to_string());
                continue;
            }
            asks |= question;
            out.choices.push(line.to_string());
        }
    }
    if out.choices.is_empty() || (ctx.decided && !asks) {
        return None;
    }
    if out.choices.is_empty() {
        return None;
    }
    // Undone checks anywhere in the reply feed the hidden round; they don't hold a reply alone
    out.unchecked = lines
        .iter()
        .filter(|(l, _)| UNCHECKED.iter().any(|u| l.to_lowercase().contains(u)))
        .map(|(l, _)| l.to_string())
        .take(8)
        .collect();
    Some(out)
}

/// Whether the person's message asks to make the choice themselves.
pub fn wants_to_pick(person_text: &str) -> bool {
    let t = person_text.to_lowercase().replace('’', "'");
    WANTS_TO_PICK.iter().any(|p| t.contains(p))
}

// The [DECISIONS] block

/// One choice a bot made, as kept in `meta.decisions`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct Decision {
    pub index: usize,
    pub question: String,
    pub options: Vec<String>,
    pub picked: String,
    pub why: String,
    pub status: String,
    pub changed_to: Option<String>,
}

pub const MAX_DECISIONS: usize = 6;
const OPEN: &str = "[DECISIONS]";
const CLOSE: &str = "[/DECISIONS]";

fn field<'a>(part: &'a str, name: &str) -> Option<&'a str> {
    let p = part.trim();
    let (k, v) = p.split_once(':')?;
    (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim())
}

fn parse_line(line: &str) -> Option<(String, Vec<String>, String, String)> {
    let line = line.trim().trim_start_matches(['-', '*', '•']).trim();
    let parts: Vec<&str> = line.split('|').collect();
    let get = |name: &str| parts.iter().find_map(|p| field(p, name));
    let question = get("question")?.to_string();
    let options: Vec<String> = get("options")?.split(';').map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
    let picked_raw = get("picked")?;
    let why = get("why").unwrap_or("").to_string();
    if question.is_empty() || !(2..=6).contains(&options.len()) {
        return None;
    }
    // The pick must be one of the options; kept as the option's exact text
    let norm = |s: &str| s.trim().trim_matches(['"', '“', '”', '.']).to_lowercase();
    let picked = options.iter().find(|o| norm(o) == norm(picked_raw))?.clone();
    Some((question, options, picked, why))
}

/// Takes a `[DECISIONS] … [/DECISIONS]` block off the end of a reply: the reply without it, and
/// its valid lines (`question: … | options: A; B | picked: B | why: …`). A reply without the
/// block comes back as it was.
pub fn take_decisions(text: &str) -> (String, Vec<Decision>) {
    let Some(start) = text.rfind(OPEN) else {
        return (text.to_string(), Vec::new());
    };
    let body_start = start + OPEN.len();
    let (body, rest) = match text[body_start..].find(CLOSE) {
        Some(end) => (&text[body_start..body_start + end], &text[body_start + end + CLOSE.len()..]),
        None => (&text[body_start..], ""),
    };
    let decisions = body
        .lines()
        .filter_map(parse_line)
        .take(MAX_DECISIONS)
        .enumerate()
        .map(|(index, (question, options, picked, why))| Decision {
            index,
            question,
            options,
            picked,
            why,
            status: "decided".into(),
            changed_to: None,
        })
        .collect();
    let stripped = format!("{}{}", text[..start].trim_end(), if rest.trim().is_empty() { "" } else { "\n\n" });
    (format!("{stripped}{}", rest.trim()).trim_end().to_string(), decisions)
}

/// `meta` with `decisions` set (when there are any).
pub fn with_decisions(meta: Option<Value>, decisions: &[Decision]) -> Option<Value> {
    if decisions.is_empty() {
        return meta;
    }
    let mut meta = meta.unwrap_or_else(|| json!({}));
    meta["decisions"] = json!(decisions);
    Some(meta)
}

// The hidden round

/// A turn the person didn't start (a discussion round or one kwnt runs on its own), or one
/// already inside the gate.
pub(super) fn system_turn(opts: &TurnOpts) -> bool {
    opts.discussion.is_some() || opts.label.is_some() || opts.always_ask || opts.hook.is_some()
}

/// Whether a turn may have its reply held: the setting is on, it's a turn the person started
/// (not a system turn or a delegation), the chain hasn't been through the gate yet, and the
/// person didn't just say they want to pick. `system` is [`system_turn`] of the turn's options (read before any await: they aren't Sync).
/// `Some` with the person's last message when it may.
pub(super) async fn eligible(scope: &TurnScope, kwant: &Kwant, system: bool) -> Option<String> {
    if system || scope.delegation_depth > 0 {
        return None;
    }
    if scope.state.turns.job_decided(&scope.job_id) || !enabled(&scope.state).await {
        return None;
    }
    let recent = scope.state.db.list_messages(&scope.context_id(kwant), 20).await.unwrap_or_default();
    let person = recent.iter().rev().find(|m| m.sender_role == "user").map(|m| m.text.clone()).unwrap_or_default();
    (!wants_to_pick(&person)).then_some(person)
}

fn event(scope: &TurnScope, kwant: &Kwant, state: &str, held_turn_id: &str, consulted: &[String], outcome: Option<&str>) {
    let _ = scope.state.event_sender.send(RuntimeEvent::DecideUpdated {
        kwant_id: kwant.id.clone(),
        context_id: Some(scope.context_id(kwant)),
        thread_id: scope.thread_id.clone(),
        state: state.into(),
        held_turn_id: held_turn_id.into(),
        consulted: consulted.to_vec(),
        outcome: outcome.map(str::to_string),
    });
}

/// Whether the chain's budget leaves room for the hidden round.
async fn fits_budget(scope: &TurnScope) -> bool {
    let budget = super::budget::limit(&scope.state).await;
    budget == 0 || scope.state.turns.job_tokens(&scope.job_id).saturating_add(RESERVE_TOKENS) <= budget
}

/// Which of `names` to ask, as indexes: the one the open choices or undone checks name (the latest
/// such line wins), else the one the reply names last. Two only when the choices and undone checks
/// name two, at most [`MAX_CONSULTS`].
pub fn pick_consultees(names: &[&str], text: &str, gate: &Gate) -> Vec<usize> {
    let named = |line: &str| -> Vec<(usize, usize)> {
        // "Pia's" names Pia
        let ws: Vec<String> = words(&line.to_lowercase().replace('’', "'"))
            .into_iter()
            .map(|w| w.strip_suffix("'s").map(str::to_string).unwrap_or(w))
            .collect();
        names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let parts = words(&n.to_lowercase());
                (!parts.is_empty()).then_some(())?;
                ws.windows(parts.len()).rposition(|w| w == parts.as_slice()).map(|pos| (i, pos))
            })
            .collect()
    };
    // Latest mention first in each tier
    let mut focus: Vec<usize> = Vec::new();
    for line in gate.choices.iter().chain(gate.unchecked.iter()).rev() {
        let mut hits = named(line);
        hits.sort_by_key(|&(_, pos)| std::cmp::Reverse(pos));
        for (i, _) in hits {
            if !focus.contains(&i) {
                focus.push(i);
            }
        }
    }
    if !focus.is_empty() {
        focus.truncate(MAX_CONSULTS);
        return focus;
    }
    let mut hits = named(text);
    hits.sort_by_key(|&(_, pos)| std::cmp::Reverse(pos));
    hits.into_iter().map(|(i, _)| i).take(1).collect()
}

/// Teammates to ask about the held reply (in a channel), besides the speaker (see [`pick_consultees`]).
fn consultees<'a>(scope: &'a TurnScope, speaker: &Kwant, text: &str, gate: &Gate) -> Vec<&'a Kwant> {
    if scope.channel_id.is_none() {
        return Vec::new();
    }
    let others: Vec<&Kwant> = scope.teammates.iter().filter(|k| k.id != speaker.id).collect();
    let names: Vec<&str> = others.iter().map(|k| k.name.as_str()).collect();
    pick_consultees(&names, text, gate).into_iter().map(|i| others[i]).collect()
}

fn bullet_list(items: &[String]) -> String {
    items.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
}

fn consult_prompt(speaker: &str, held: &str) -> String {
    format!(
        "[Decide first] {speaker} wrote this reply but hasn't sent it to the person, because it asks them to choose. \
         The team decides reversible choices itself.\n---\n{held}\n---\n\
         In at most 6 lines: for each open choice, which option you would pick and why, from what you know or can check \
         quickly. Don't @mention anyone and don't ask the person anything."
    )
}

/// The prompt of the hidden round.
pub fn decide_prompt(held: &str, gate: &Gate, views: &[(String, String)]) -> String {
    let mut p = format!(
        "[Decide first] Your last reply hasn't reached the person. It ends by asking them to choose, and the person \
         wants the team to decide reversible choices itself instead of asking.\nYour held reply:\n---\n{held}\n---\n"
    );
    if !gate.choices.is_empty() {
        p.push_str(&format!("Open choices:\n{}\n", bullet_list(&gate.choices)));
    }
    if !gate.unchecked.is_empty() {
        p.push_str(&format!("Your reply says these weren't checked:\n{}\n", bullet_list(&gate.unchecked)));
    }
    if !gate.kept.is_empty() {
        p.push_str(&format!("These stay with the person (they can't be undone or reach outside):\n{}\n", bullet_list(&gate.kept)));
    }
    for (name, view) in views {
        p.push_str(&format!("{name}'s view:\n{}\n", view.trim()));
    }
    p.push_str(
        "Now:\n\
         1. Do the checks your reply lists as not done, where you can (read the files, open the pages, run the tools).\n\
         2. Decide every reversible choice yourself, from what the person asked, what you found, your teammates' views and \
         [How the person usually decides] if you have it.\n\
         3. Apply what you decided when it can be undone. Never send, publish, pay, contact anyone, delete or sign in for this.\n\
         4. Write the reply the person sees instead of the held one: what you decided and why, what you checked, and that \
         they can change it. Keep a question only when it really needs the person (it can't be undone, costs money, reaches \
         someone outside, needs a password or sign-in, or nothing you can find answers it): ask it once, with ask_user, \
         saying why only they can answer and what you checked, and do everything that doesn't wait on the answer first \
         (a draft with a placeholder, for example). Don't add optional questions or offers.\n\
         End with one line per choice you made:\n\
         [DECISIONS]\n\
         question: <the choice> | options: <option 1>; <option 2>; <option 3> | picked: <the exact option> | why: <one line>\n\
         [/DECISIONS]",
    );
    p
}

/// Saves the held reply as it was and says so: the gate couldn't finish.
async fn post_original(scope: &TurnScope, kwant: &Kwant, held: Held, consulted: &[String], outcome: &str) -> String {
    let mut meta = held.meta.unwrap_or_else(|| json!({}));
    meta["decide_first"] = json!({"consulted": consulted, "held_turn_id": held.turn_id, "kwant_id": kwant.id, "outcome": outcome});
    if let Some(saved) = scope.save(&held.msg, Some(meta)).await {
        let _ = scope.state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    }
    super::waiting::turn_ended(scope, kwant, &held.turn_id, &held.text).await;
    event(scope, kwant, "done", &held.turn_id, consulted, Some(outcome));
    held.text
}

/// Runs the hidden round for a held reply and returns the text that reached the person (the
/// decided reply, or the held one when the round couldn't run or finish).
pub(super) async fn decide(scope: &TurnScope, kwant: &Kwant, mut held: Held) -> String {
    scope.state.turns.mark_decided(&scope.job_id);
    let gate = std::mem::take(&mut held.gate);
    // The bot already asked with a card in this turn: that ask is the person's, not the round's
    let context = scope.context_id(kwant);
    let asked = scope.state.db.list_questions(None, Some(&context), None).await.unwrap_or_default();
    if asked.iter().any(|q| q.turn_id.as_deref() == Some(held.turn_id.as_str())) {
        return post_original(scope, kwant, held, &[], "posted_original").await;
    }
    let consult: Vec<Kwant> = consultees(scope, kwant, &held.text, &gate).into_iter().cloned().collect();
    let names: Vec<String> = consult.iter().map(|k| k.name.clone()).collect();
    event(scope, kwant, "deciding", &held.turn_id, &names, None);
    if !fits_budget(scope).await {
        return post_original(scope, kwant, held, &[], "posted_original").await;
    }
    let mut views = Vec::new();
    for k in &consult {
        let mut opts = TurnOpts::default();
        (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(PURPOSE.into()));
        if let Ok(view) = turn_once(scope, k, consult_prompt(&kwant.name, &held.text), opts).await {
            views.push((k.name.clone(), view));
        }
        if scope.cancel_token.is_cancelled() {
            return post_original(scope, kwant, held, &names, "cancelled").await;
        }
    }
    let consulted: Vec<String> = views.iter().map(|(n, _)| n.clone()).collect();
    if !fits_budget(scope).await {
        return post_original(scope, kwant, held, &consulted, "posted_original").await;
    }
    let hook = HookSlot::default();
    let mut opts = TurnOpts::default();
    (opts.quiet_failure, opts.purpose, opts.hook) = (true, Some(PURPOSE.into()), Some(hook.clone()));
    opts.extra_meta = Some(json!({"decide_first": {"consulted": consulted, "held_turn_id": held.turn_id, "kwant_id": kwant.id}}));
    let reply = turn_once(scope, kwant, decide_prompt(&held.text, &gate, &views), opts).await;
    let saved = hook.lock().ok().and_then(|mut h| h.saved.take());
    match (reply, saved) {
        (Ok(text), Some(saved)) => {
            let _ = scope.state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
            event(scope, kwant, "done", &held.turn_id, &consulted, Some("decided"));
            text
        }
        _ if scope.cancel_token.is_cancelled() => post_original(scope, kwant, held, &consulted, "cancelled").await,
        _ => post_original(scope, kwant, held, &consulted, "posted_original").await,
    }
}

fn meta_of(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

fn decisions_of(meta: &Value) -> Vec<Decision> {
    serde_json::from_value(meta["decisions"].clone()).unwrap_or_default()
}

/// Keeps a note of each choice a bot made in `saved` (for the keep rate).
pub(super) async fn note_bot_decisions(state: &AppState, kwant: &Kwant, saved: &Message) {
    let space = SpaceId::new(&kwant.space_id);
    for d in decisions_of(&meta_of(saved)) {
        let _ = state
            .db
            .add_decision_note(&NewDecisionNote {
                space: &space,
                kwant_id: Some(&kwant.id),
                message_id: Some(&saved.id),
                question: &d.question,
                options: &d.options,
                picked: &d.picked,
                bot_pick: None,
                source: "bot",
            })
            .await;
    }
}

/// Keeps a note of the person's answer to a question card with choices.
pub(super) async fn note_answer(state: &AppState, q: &crate::db::questions::Question) {
    let options = q.choice_list();
    let Some(answer) = q.answer.as_deref() else { return };
    if options.len() < 2 {
        return;
    }
    let Ok(Some(space)) = state.db.space_of_kwant(&q.kwant_id).await else { return };
    let _ = state
        .db
        .add_decision_note(&NewDecisionNote {
            space: &space,
            kwant_id: Some(&q.kwant_id),
            message_id: q.message_id.as_deref(),
            question: &q.question,
            options: &options,
            picked: answer,
            bot_pick: None,
            source: "answered",
        })
        .await;
}

// Change

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePayload {
    pub picked: String,
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

/// The person's message that asks the bot to redo a decision.
pub fn change_prompt(bot: Option<&str>, d: &Decision, picked: &str) -> String {
    let mention = bot.map(|b| format!("@{b} ")).unwrap_or_default();
    format!("{mention}Change of decision: for “{}” use “{picked}” instead of “{}”. Redo what depended on it.", d.question, d.picked)
}

/// `POST /api/messages/:message_id/decisions/:index/change` (person-only): marks a bot's choice as
/// changed, keeps a note of it, and returns `{message, prompt}`; the client then sends `prompt` to
/// the same chat.
pub async fn change(
    State(state): State<AppState>,
    Path((message_id, index)): Path<(String, usize)>,
    payload: Result<Json<ChangePayload>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(p) = match payload {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, e.body_text()),
    };
    let msg = match state.db.get_message(&message_id).await {
        Ok(Some(m)) if m.sender_role == "kwant" => m,
        Ok(_) => return error(StatusCode::NOT_FOUND, "no such message"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "could not load the message"),
    };
    let mut meta = meta_of(&msg);
    let mut decisions = decisions_of(&meta);
    let Some(d) = decisions.iter_mut().find(|d| d.index == index) else {
        return error(StatusCode::NOT_FOUND, "no such decision");
    };
    if d.status == "changed" {
        return error(StatusCode::CONFLICT, "that decision was already changed");
    }
    let picked = p.picked.trim();
    let Some(picked) = d.options.iter().find(|o| o.as_str() == picked).cloned() else {
        return error(StatusCode::BAD_REQUEST, "picked must be one of the options");
    };
    if picked == d.picked {
        return error(StatusCode::BAD_REQUEST, "that is already the pick");
    }
    let before = d.clone();
    d.status = "changed".into();
    d.changed_to = Some(picked.clone());
    meta["decisions"] = json!(decisions);
    if state.db.set_message_meta(&message_id, &meta).await.is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the change");
    }
    let Ok(Some(updated)) = state.db.get_message(&message_id).await else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "could not load the message");
    };
    let context = msg.thread_id.as_deref().or(msg.channel_id.as_deref()).or(msg.direct_kwant_id.as_deref()).unwrap_or("");
    if let Ok(Some(space)) = state.db.space_of_context(context).await {
        let kwant_id = meta["decide_first"]["kwant_id"].as_str().map(str::to_string);
        let _ = state
            .db
            .add_decision_note(&NewDecisionNote {
                space: &space,
                kwant_id: kwant_id.as_deref(),
                message_id: Some(&message_id),
                question: &before.question,
                options: &before.options,
                picked: &picked,
                bot_pick: Some(&before.picked),
                source: "changed",
            })
            .await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&updated).unwrap_or(Value::Null) });
    // In a channel the prompt goes to the bot that decided
    let bot = msg.channel_id.as_ref().map(|_| msg.sender_id.as_str());
    Json(json!({"message": updated, "prompt": change_prompt(bot, &before, &picked)})).into_response()
}

// "How the person usually decides"

/// Notes shown in the prompt block at most.
pub const NOTES_SHOWN: i64 = 15;
/// The block's size cap, in bytes.
pub const MAX_BLOCK_BYTES: usize = 2000;

fn note_line(n: &DecisionNote) -> String {
    let opts = n.option_list().join(" / ");
    match (n.source.as_str(), &n.bot_pick) {
        ("changed", Some(bot)) => format!("- “{}” ({opts}): changed the bot's “{bot}” to “{}”", n.question, n.picked),
        _ => format!("- “{}” ({opts}): picked “{}”", n.question, n.picked),
    }
}

/// The block from the space's notes, at most [`MAX_BLOCK_BYTES`]; `None` without notes.
pub fn block_from(person: &str, notes: &[DecisionNote], kept: (i64, i64)) -> Option<String> {
    if notes.is_empty() && kept.1 == 0 {
        return None;
    }
    let mut out = format!(
        "\n\n[How {person} usually decides]\nPredict their choice from these and act on it; say \"predicted from your earlier choices\" \
         and that they can change it, instead of asking.\n"
    );
    if kept.1 > 0 {
        out.push_str(&format!("They kept {} of the bots' last {} own choices.\n", kept.0, kept.1));
    }
    for n in notes {
        let line = note_line(n);
        if out.len() + line.len() + 1 > MAX_BLOCK_BYTES {
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// The prompt block for `space` (empty without notes or with decide-first off).
pub async fn prompt_block(state: &AppState, space: &SpaceId) -> String {
    if !enabled(state).await {
        return String::new();
    }
    let notes = state.db.list_decision_notes(space, &["answered", "changed"], NOTES_SHOWN).await.unwrap_or_default();
    let kept = state.db.decision_keep_rate(space, 20).await.unwrap_or((0, 0));
    block_from("the person", &notes, kept).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oleks's screenshot: the lead's reply that asked him to pick.
    const SCREENSHOT: &str = "Here's where the three directions stand.\n\n\
        **A: Classic** keeps the current layout with a serif title.\n\
        **B: Wax seal** puts the seal on the hero; Designer mocked it up and it reads well.\n\
        **C: Green desk** uses the dark green palette.\n\n\
        Status: Designer only looked at B; nobody looked at A and C; nobody ran a contrast checker; C's focus rings unchecked. \
        This used about 40k tokens so far.\n\n\
        Two decisions for you:\n\
        1. A, B or C?\n\
        2. FM monogram or plain?\n\n\
        I'll hold the build until you answer.";

    #[test]
    fn the_screenshot_reply_is_held_with_its_undone_checks() {
        let g = gate(SCREENSHOT).expect("held");
        assert!(g.choices.iter().any(|c| c.contains("A, B or C")));
        assert!(g.choices.iter().any(|c| c.contains("monogram")));
        assert!(g.unchecked.iter().any(|c| c.contains("nobody looked at A and C")));
    }

    #[test]
    fn an_ask_about_a_step_that_cant_be_undone_is_not_held() {
        assert_eq!(gate("The draft is ready.\n\nShould I send the email to Mia now, or wait until Monday?"), None);
        assert_eq!(gate("Which plan should I buy: Pro or Team?"), None);
        assert_eq!(gate("Want me to delete the old branch or keep it?"), None);
        assert_eq!(gate("Should I sign in with your Google account or the work one?"), None);
    }

    #[test]
    fn replies_without_an_open_choice_are_not_held() {
        assert_eq!(gate("Done: the page is live on staging and the tests pass."), None);
        assert_eq!(gate("I fixed it.\n\nAnything else?"), None);
        assert_eq!(gate("I fixed it. Let me know if you need anything else."), None);
        assert_eq!(gate("Not checked yet: the dark mode contrast. I'll run it next."), None);
    }

    #[test]
    fn other_choice_shapes_are_held() {
        assert!(gate("Two headlines are ready.\nDo you prefer the short one or the long one?").is_some());
        assert!(gate("Pricing could be $9 or $12.\nYour call.").is_some());
        assert!(gate("Which direction do you want for the logo?").is_some());
    }

    /// s4: the person asked to send an email; the reply can't send it, and offers a subject.
    const S4: &str = "I haven't sent it. I have no way to send email from here.\n\n\
        I've put an install card in the chat for a Gmail connector. If you use Gmail, install it and tell me. \
        I'll send the message from your account on my next turn.\n\n\
        Two things to check before it goes out. Tuesday is tomorrow (27 Oct). The message also has no subject line, \
        so say if you want me to add a subject such as \"Folio Mill launches Tuesday: early look?\" and a name at the end.";

    #[test]
    fn soft_offers_are_held() {
        let g = gate_in(S4, GateCtx { person_hard: true, decided: false }).expect("the subject offer is held");
        assert!(g.choices.iter().any(|c| c.contains("add a subject")), "{g:?}");
        assert!(g.kept.iter().any(|c| c.contains("install it and tell me")), "the send stays with the person: {g:?}");
        assert!(gate("I created styles/cta.css in orange.\n\nIf your real stylesheet lives somewhere else, tell me the path and I'll move the rule there.").is_some());
        assert!(gate("Orange is in.\nLet me know if you'd like me to switch it to teal.").is_some());
        assert!(gate("Orange is in. If you'd rather have teal, it's a one-line swap.").is_some());
        assert!(gate("The draft is saved. Tell me which headline and I'll finish the page.").is_some());
        assert!(gate("Want me to add a dark mode too?").is_some());
    }

    #[test]
    fn closers_and_plain_replies_are_not_soft_offers() {
        assert_eq!(gate("Done."), None);
        assert_eq!(gate("Done. Let me know if you need anything else."), None);
        assert_eq!(gate("Done. Tell me if anything looks off and I'll fix it."), None);
        assert_eq!(gate("The draft is ready. Should I send it?"), None);
        assert_eq!(gate("The draft is ready. Tell me when to send it and I'll do it."), None);
    }

    #[test]
    fn a_question_about_the_hard_step_the_person_asked_for_is_not_held() {
        // s10: "Sign us up for the Pro plan"
        let reply = "Paying and signing in are yours to do.\n\n\
            The likely options are Plus at $4.99/month or at $49.99/year.\n\n\
            Should I stop at confirm for that, or is it the annual plan you meant?";
        assert!(person_asked_hard("Sign us up for the 49 USD/month Pro plan on StoryGraph"));
        assert_eq!(gate_in(reply, GateCtx { person_hard: true, decided: false }), None);
        // The line before a choice in its paragraph counts too
        assert_eq!(gate("You sign in and pay.\nMonthly or yearly?"), None);
        assert!(gate("You sign in and pay.\n\nFor the report, monthly or yearly charts?").is_some());
    }

    #[test]
    fn asking_the_person_for_a_fact_is_not_a_hard_step() {
        // s9: only the person has the VAT ID; the draft-or-wait offer is the bot's to decide
        let reply = "I couldn't find Northwind Books' VAT ID.\n\nCould you send me the VAT ID?\n\n\
            If you'd rather not wait, tell me and I'll save a draft with [VAT ID TBD] placeholders.";
        let g = gate(reply).expect("held");
        assert_eq!(g.choices.len(), 1, "{g:?}");
        assert!(g.kept.is_empty(), "{g:?}");
    }

    #[test]
    fn a_reply_that_already_decided_is_held_only_for_an_open_question() {
        let decided = GateCtx { person_hard: false, decided: true };
        assert_eq!(gate_in("I went with orange. If you'd rather have teal, it's a one-line swap.", decided), None);
        assert!(gate_in("I went with orange.\n\nShould the hover be darker or lighter?", decided).is_some());
    }

    #[test]
    fn one_teammate_is_asked_unless_the_choices_name_two() {
        let names = ["Pia", "Ana", "Wren"];
        let g = gate(SCREENSHOT.replace("Designer only looked at B", "Pia only looked at B").as_str()).unwrap();
        assert_eq!(pick_consultees(&names, "Pia and Ana and Wren worked on it", &g), vec![0]);
        let g = Gate { choices: vec!["Pia's B or Ana's C?".into()], ..Default::default() };
        assert_eq!(pick_consultees(&names, "", &g), vec![1, 0]);
        let g = Gate { choices: vec!["A or B?".into()], ..Default::default() };
        assert_eq!(pick_consultees(&names, "Pia drew it, then Wren wrote the copy.\nA or B?", &g), vec![2]);
        assert_eq!(pick_consultees(&names, "A or B?", &g), Vec::<usize>::new());
    }

    #[test]
    fn an_offer_naming_a_teammate_is_not_a_hand_on() {
        let names = ["Pia", "Ana"];
        assert!(!hands_on("A, B or C?\n\nI can have @Pia screenshot A and C before you decide. Do you want that?", &names));
        assert!(hands_on("Done on my side.\n@Pia, can you screenshot A and C?", &names));
        assert!(hands_on("Done on my side.\n- @Ana: run the contrast check", &names));
        assert!(hands_on("@Pia please screenshot A and C.", &names));
    }

    #[test]
    fn the_person_asking_to_pick_is_seen() {
        assert!(wants_to_pick("Give me options for the hero, let me pick"));
        assert!(wants_to_pick("Show me the options; I’ll decide"));
        assert!(!wants_to_pick("Make the hero nicer"));
    }

    #[test]
    fn the_decisions_block_is_taken_off_and_parsed() {
        let text = "I went with B.\n\n[DECISIONS]\n\
            question: Which design direction? | options: A: Classic; B: Wax seal; C: Green desk | picked: b: wax seal | why: matches the seal\n\
            question: Monogram? | options: FM monogram; plain | picked: plain | why: cleaner\n\
            question: bad | options: only one | picked: only one | why: x\n\
            question: Off list | options: X; Y | picked: Z | why: x\n\
            [/DECISIONS]";
        let (stripped, ds) = take_decisions(text);
        assert_eq!(stripped, "I went with B.");
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].picked, "B: Wax seal");
        assert_eq!(ds[0].options.len(), 3);
        assert_eq!((ds[1].index, ds[1].picked.as_str(), ds[1].status.as_str()), (1, "plain", "decided"));
        assert_eq!(take_decisions("no block").0, "no block");
    }

    #[test]
    fn the_change_prompt_names_the_bot_in_channels() {
        let d = Decision {
            index: 0,
            question: "Which design direction?".into(),
            options: vec!["A: Classic".into(), "B: Wax seal".into()],
            picked: "B: Wax seal".into(),
            why: String::new(),
            status: "decided".into(),
            changed_to: None,
        };
        assert_eq!(
            change_prompt(Some("Designer"), &d, "A: Classic"),
            "@Designer Change of decision: for “Which design direction?” use “A: Classic” instead of “B: Wax seal”. Redo what depended on it."
        );
        assert!(change_prompt(None, &d, "A: Classic").starts_with("Change of decision"));
    }

    #[test]
    fn the_block_is_capped() {
        let note = |i: usize| DecisionNote {
            id: i.to_string(),
            space_id: "s".into(),
            kwant_id: None,
            message_id: None,
            question: "q".repeat(250),
            options: serde_json::to_string(&["a".repeat(100), "b".repeat(100)]).unwrap(),
            picked: "a".repeat(100),
            bot_pick: None,
            source: "answered".into(),
            created_at: chrono::Utc::now(),
        };
        let notes: Vec<_> = (0..15).map(note).collect();
        let block = block_from("the person", &notes, (3, 4)).unwrap();
        assert!(block.len() <= MAX_BLOCK_BYTES, "{}", block.len());
        assert!(block.contains("kept 3 of the bots' last 4"));
        assert_eq!(block_from("the person", &[], (0, 0)), None);
    }
}
