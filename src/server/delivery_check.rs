//! The delivery check (0.8.0 P4, P9 and P10): after a turn, compare what the person expected to
//! see in the chat with what the turn really did, before its reply is saved.
//!
//! - A reply that claims a picture ("Here is the mockup…") when the turn shared none gets the
//!   turn's last screenshot shared under it, or [`NO_PICTURE_LINE`](super::picture_claims::NO_PICTURE_LINE) when there is none (P4; the
//!   claim matcher is [`super::picture_claims`]).
//! - A turn whose plan expected an image or a link and that delivered neither gets one automatic
//!   continuation naming exactly what is missing (P9, P10). A continuation never continues again.
//!   If it still shares no image, its reply gets [`NO_PICTURE_LINE`](super::picture_claims::NO_PICTURE_LINE).
//!   Only a firm plan continues ([`DeliveryPlan::firm`]): a lean one ("@Ann look at this") is
//!   recorded as a hit or a miss and gets the no-picture line only when the reply claims a picture.
//! - The result is recorded as a delivery hit or miss ([`record_delivery_note`]).
//! - Follow-through (0.9.4): a turn whose last tool step failed or was refused and whose reply
//!   ends on a plan it didn't carry out ("I'll check the file sizes next.", see [`promise_line`])
//!   gets the same one continuation, told to retry, try another way, or end with a
//!   `Blocked: <what>. <what unblocks it>` line. It needs no plan. If the continuation still ends
//!   on a promise after a failed step, its reply gets [`follow_line`], and (22j) the save step
//!   stores kwant's own [`blocked_text`] on it, so the lead's chain is Blocked at once (one card
//!   naming the step and its error) instead of looping through keep-going turns. A step the
//!   person declined on an approval card is never pushed again. Follow-through is never a
//!   delivery note.
//!
//! What the person expected comes from the turn's one `DeliveryPlan` (`src/router/delivery.rs`),
//! the same plan the prompt's "[What to deliver]" hint renders, read through [`Expect::from_plan`].
//! The continuation carries that plan; it never makes a new one from kwant's own prompt.

use super::picture_claims::claims_picture;
use crate::router::delivery::{DeliveryPlan, Show};
use serde_json::Value;

/// What the person expects to see in the chat for one turn: the parts of the turn's
/// `DeliveryPlan` this check can verify. `image` is `plan.show` containing `Show::Image`, `link`
/// is `plan.show` containing `Show::Link`, `done_when` is `plan.done_when` and `firm` is `plan.firm`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expect {
    pub image: bool,
    pub link: bool,
    pub done_when: Vec<String>,
    /// The person clearly asked ([`DeliveryPlan::firm`]): only then does a miss continue.
    pub firm: bool,
}

impl Expect {
    /// What the turn's plan lets this check verify: `image` from [`DeliveryPlan::wants_picture`],
    /// `link` from `Show::Link`, the plan's `done_when` and `firm`. `None` when it wants neither
    /// (Explain and Other plans), so those turns are not continued or recorded.
    pub fn from_plan(plan: &DeliveryPlan) -> Option<Self> {
        let (image, link) = (plan.wants_picture(), plan.show.contains(&Show::Link));
        (image || link).then(|| Expect { image, link, done_when: plan.done_when.clone(), firm: plan.firm })
    }
}

/// What the turn really did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    /// It put an image into the chat (`share_file`, `browser_screenshot` with `share: true`).
    pub image_shared: bool,
    /// It took a screenshot (the one the guard can share).
    pub has_shot: bool,
    /// It pushed a commit or opened a PR (see [`pushed_in`]).
    pub pushed: bool,
    /// It ran at least one tool step (see [`tool_steps_in`]).
    pub ran_tools: bool,
    /// Its last tool step, when that one failed or was refused (see [`tool_steps_in`]). A later
    /// tool step that worked means the bot moved on: then this is `None`.
    pub failed_step: Option<FailedStep>,
    /// The person declined an approval card in it: that step is never pushed again.
    pub declined: bool,
}

/// A tool step that failed or was refused: the tool's name, the step's short title and (22j) its
/// `detail`, which for a failed step is why it failed (see [`crate::drivers::activity`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedStep {
    pub tool: String,
    pub title: String,
    pub detail: String,
}

/// Longest error quoted from a failed step's detail (characters).
const MAX_ERROR_CHARS: usize = 160;

impl FailedStep {
    /// What the step did, for a sentence: its title without the "Running "/"Using " kwant puts in
    /// front ("`journalctl -u bot`"), or the tool's name when it has no title.
    pub fn name(&self) -> String {
        let title = self.title.trim();
        let bare = ["Running ", "Using "].iter().find_map(|p| title.strip_prefix(p)).unwrap_or(title).trim();
        if bare.is_empty() { self.tool.clone() } else { clip(bare, MAX_QUOTE_CHARS) }
    }

    /// The first non-empty line of the step's detail (why it failed), at most
    /// [`MAX_ERROR_CHARS`], without a closing full stop. `None` when it has none.
    pub fn error(&self) -> Option<String> {
        let line = self.detail.lines().map(str::trim).find(|l| !l.is_empty())?;
        let line = cut_at_word(line, MAX_ERROR_CHARS);
        let line = line.trim_end_matches(['.', ' ']);
        (!line.is_empty()).then(|| line.to_string())
    }
}

/// 22j: kwant's own `meta.blocked` text for `bot`'s reply that still ended on a promise after
/// `step` failed: "<Bot> couldn't run <step>: <error>." It becomes the Blocked card's question
/// ("Blocked: …", see [`super::keep_going`]).
pub fn blocked_text(bot: &str, step: &FailedStep) -> String {
    match step.error() {
        Some(error) if error.ends_with('…') => format!("{bot} couldn't run {}: {error}", step.name()),
        Some(error) => format!("{bot} couldn't run {}: {error}.", step.name()),
        None => format!("{bot} couldn't run {}.", step.name()),
    }
}

impl Seen {
    /// This turn's `self` counted with what its delivery's earlier turns did (`so_far`, which it
    /// then joins): an image shared or a push made by any turn of the delivery counts (0.8.1 F1).
    /// `has_shot` stays this turn's own: only its own screenshot can be shared under its reply.
    /// The last tool step is the delivery's last: a turn that ran no tool (a continuation that
    /// only promised again) still ends after the earlier turn's failed step, and a turn that ran
    /// tools has its own. A declined card in any of its turns counts.
    pub fn with_earlier(self, so_far: &mut Seen) -> Seen {
        so_far.image_shared |= self.image_shared;
        so_far.pushed |= self.pushed;
        so_far.declined |= self.declined;
        if self.ran_tools {
            so_far.ran_tools = true;
            so_far.failed_step = self.failed_step;
        }
        Seen {
            image_shared: so_far.image_shared,
            has_shot: self.has_shot,
            pushed: so_far.pushed,
            ran_tools: so_far.ran_tools,
            failed_step: so_far.failed_step.clone(),
            declined: so_far.declined,
        }
    }
}

/// Something the person expected that the turn didn't deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// No image in the chat.
    Image,
    /// No PR or branch link in the reply, and nothing pushed.
    Link,
    /// The last tool step failed or was refused, and the reply ends on a plan it didn't carry
    /// out. Never a delivery note (see [`shows`]).
    FollowThrough,
}

/// A delivery note's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Hit,
    Miss,
}

/// What to do with one reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Share the turn's last screenshot under the reply.
    pub share_last_shot: bool,
    /// Add [`NO_PICTURE_LINE`](super::picture_claims::NO_PICTURE_LINE) to the reply.
    pub no_picture_line: bool,
    /// What the turn didn't deliver (after the automatic share).
    pub missing: Vec<Missing>,
    /// The prompt of the one continuation, when something is missing and this turn isn't the
    /// continuation already.
    pub continue_with: Option<String>,
    /// The delivery note to record: none without a plan, nor while a continuation decides it.
    pub outcome: Option<Outcome>,
    /// Add [`follow_line`] for this tool: the continuation still ended on a promise after a
    /// failed step.
    pub follow_line: Option<String>,
    /// The failed step behind [`Verdict::follow_line`] (22j): the save step stores
    /// [`blocked_text`] for it, so the chain is Blocked, not stalled.
    pub stuck_on: Option<FailedStep>,
}

/// The P9 continuation for a missing image (Ada's wording).
pub const IMAGE_PROMPT: &str = "You were asked for something to see. Render it (use the project's own CSS for a mockup), \
take a screenshot with share: true, and reply.";

/// The P10 continuation for a missing link.
pub const LINK_PROMPT: &str = "You were asked for a change, and your reply has no link to it. Commit and push your work, \
then reply with the PR or branch link.";

/// Longest promise or step title quoted in the follow-through prompt (characters).
const MAX_QUOTE_CHARS: usize = 200;

/// The follow-through continuation (0.9.4): what failed and what the reply promised, then the
/// rule. Ada's wording.
pub fn follow_prompt(step: &FailedStep, promise: &str) -> String {
    let title = clip(step.title.trim(), MAX_QUOTE_CHARS);
    let what = if title.is_empty() || title == step.tool { step.tool.clone() } else { format!("{}: {title}", step.tool) };
    format!(
        "Your last step ({what}) failed or was refused, and your reply ends with a plan that didn't run (\"{}\"). \
Do it now: retry once if it may have been a fluke, then try another way (another tool or command, reading instead of \
running, your bot browser, or a teammate who can). If nothing works, end with one line \
`Blocked: <what is blocked>. <what would unblock it>`.",
        clip(promise.trim(), MAX_QUOTE_CHARS)
    )
}

/// Added to a continuation's reply that still ends on a promise after a failed step: the
/// continuation was the other try, so the line says the step failed again (22j).
pub fn follow_line(tool: &str, bot: &str) -> String {
    format!("_kwant: {tool} failed again, and {bot} didn't try another way or say why._")
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Most `done_when` checks quoted in a continuation, and the longest one (characters).
const MAX_CHECKS: usize = 5;
const MAX_CHECK_CHARS: usize = 200;

/// Checks one reply. `continued` says the turn is the one continuation already. A lean `expect`
/// never continues and adds the no-picture line only for a claimed picture. Follow-through
/// continues with or without a plan (see the module docs).
pub fn check(reply: &str, expect: Option<&Expect>, continued: bool, seen: &Seen) -> Verdict {
    let claims = claims_picture(reply);
    let wants_image = expect.is_some_and(|e| e.image);
    let mut v = Verdict::default();
    let mut image = seen.image_shared;
    // A claimed picture, or the continuation that took a screenshot but didn't share it
    if !image && seen.has_shot && (claims || (wants_image && continued)) {
        v.share_last_shot = true;
        image = true;
    }
    if wants_image && !image {
        v.missing.push(Missing::Image);
    }
    if expect.is_some_and(|e| e.link) && !seen.pushed && !has_change_link(reply) {
        v.missing.push(Missing::Link);
    }
    // Planned misses, before follow-through joins them: only these are a delivery note
    let planned = v.missing.clone();
    let follow = seen.failed_step.as_ref().filter(|_| !seen.declined).and_then(|step| Some((step, promise_line(reply)?)));
    if follow.is_some() {
        v.missing.push(Missing::FollowThrough);
    }
    let firm = expect.is_some_and(|e| e.firm);
    // What one continuation would ask for: a firm plan's misses, and follow-through
    let mut asks: Vec<Missing> = if firm { planned.clone() } else { vec![] };
    if follow.is_some() {
        asks.push(Missing::FollowThrough);
    }
    if !asks.is_empty() && !continued {
        let done_when = expect.map(|e| e.done_when.as_slice()).unwrap_or_default();
        v.continue_with = Some(continuation_prompt(&asks, done_when, follow));
    } else {
        v.no_picture_line = !image && (claims || (wants_image && firm));
        v.outcome = expect.map(|_| if planned.is_empty() { Outcome::Hit } else { Outcome::Miss });
        v.follow_line = follow.as_ref().map(|(step, _)| step.tool.clone());
        v.stuck_on = follow.map(|(step, _)| step.clone());
    }
    v
}

/// The continuation's prompt: what is missing, how to deliver it, and the plan's checks.
/// `follow` is the failed step and the reply's promise, for [`Missing::FollowThrough`].
pub fn continuation_prompt(missing: &[Missing], done_when: &[String], follow: Option<(&FailedStep, String)>) -> String {
    let what: Vec<&str> = missing
        .iter()
        .map(|m| match m {
            Missing::Image => "an image in the chat",
            Missing::Link => "a PR or branch link",
            Missing::FollowThrough => "another try after the failed step",
        })
        .collect();
    let mut out = format!("[kwnt: one automatic continuation. Missing from your last reply: {}.]", what.join(" and "));
    for m in missing {
        out.push(' ');
        match m {
            Missing::Image => out.push_str(IMAGE_PROMPT),
            Missing::Link => out.push_str(LINK_PROMPT),
            Missing::FollowThrough => match &follow {
                Some((step, promise)) => out.push_str(&follow_prompt(step, promise)),
                None => continue,
            },
        }
    }
    let checks: Vec<String> = done_when
        .iter()
        .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|c| !c.is_empty())
        .take(MAX_CHECKS)
        .map(|c| c.chars().take(MAX_CHECK_CHARS).collect())
        .collect();
    if !checks.is_empty() {
        out.push_str(&format!(" Done when: {}.", checks.join("; ").trim_end_matches('.')));
    }
    out
}

/// Words that open a first-person plan: "I'll check…", "Let me retry…", "I'm going to…".
const PROMISE_OPENERS: &[&str] = &["i'll ", "i will ", "let me ", "i'm going to ", "i am going to ", "going to ", "i'm gonna "];
/// Words a plan may start with before its opener: "Next, I'll…", "Now let me…".
const PROMISE_LEADS: &[&str] = &["next", "now", "then", "so", "ok", "okay", "first", "and", "after that"];
/// A last paragraph with any of these waits on the person or offers, so it isn't a promise:
/// "I'll continue once you allow Terminal", "Let me know if…", "I'll wait for your answer".
const NOT_A_PROMISE: &[&str] = &[
    "once you", "when you", "after you", "if you", "as soon as you", "until you", "let me know", "wait for you",
    "waiting for you", "wait for your", "i'll be here", "i'll wait",
];

/// The plan the reply ends on and didn't carry out, if it does (0.9.4): its last paragraph
/// (the last item of a list), markdown aside, has a sentence that opens in the first person with
/// "I'll", "I will", "Let me", "Next I'll", "Now I'll", "I'm going to" or "Going to". Not a promise:
/// a `Done:` or `Blocked:` line, a question to the person, or a line that waits on the person
/// ("once you…", "when you…", "after you…", "if you…") or offers ("Let me know…"). Conservative
/// on purpose: it only matters after a failed step. Returns that sentence.
pub fn promise_line(reply: &str) -> Option<String> {
    let para = reply.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).last()?;
    let lines: Vec<&str> = para.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let listed = |l: &str| l.starts_with(['-', '*', '+']) || l.split_once(['.', ')']).is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let last = if lines.iter().any(|l| listed(l)) { lines.last()?.to_string() } else { lines.join(" ") };
    let bare = strip_markdown(&last).replace(['’', '‘'], "'");
    // ASCII lowercase keeps byte offsets, so the sentence spans index `bare` too
    let lower = bare.to_ascii_lowercase();
    if lower.starts_with("done:") || lower.starts_with("blocked:") || lower.trim_end_matches(['*', '_', '`', ')', ' ']).ends_with('?') {
        return None;
    }
    if NOT_A_PROMISE.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let sentences = sentence_spans(&lower);
    sentences.into_iter().find_map(|(at, end)| {
        let s = lower[at..end].trim_start();
        let s = strip_leads(s);
        PROMISE_OPENERS.iter().any(|o| s.starts_with(o)).then(|| bare[at..end].trim().to_string())
    })
}

/// Plain asks of the person (13a): "Send me the URL", "paste the output", "approve the command",
/// "tell me to run it". Matched as whole words, and not in a sentence that opens with the bot's own
/// plan ("I'll share…").
const ASKS_PERSON: &[&str] = &["send me", "paste", "share", "upload", "allow", "approve", "sign in", "log in", "connect", "tell me"];
/// Offers, not waits: a reply that only offers ("Let me know if…", "If you'd like, I can…", "Tell me if
/// you want…") goes on without the person, so a part of a sentence with one of these is left out before
/// looking for a wait (see [`asks_in`]). "Tell me if/whether" only offers with "you" after it: "Tell me
/// whether to retry or skip" asks for a decision (16c).
const OFFERS: &[&str] = &[
    "let me know", "i'll be here", "if you want", "if you'd like", "if you would like", "if you like", "if you prefer",
    "if you need anything", "tell me if you", "tell me whether you",
];
/// Asks for a decision, not offers (17e): "Let me know whether to retry or skip", "Let me know which
/// file to use". A part with one of these isn't an offer even with an [`OFFERS`] word in it. Next to
/// "when you're ready" it is a go-ahead like a bare "tell me" ("Open it when you're ready and tell me
/// what to change"), so it doesn't ask there. "Let me know whether you…", "Let me know if you want…" and
/// "Let me know how it goes" stay offers. Only "whether to" asks on its own: after "which" or "what" the
/// part must name the pick ([`decides`]), so "Let me know what else you need" and "let me know what you
/// think" stay closers (19b).
const ASKS_DECISION: &[&str] = &["let me know whether to", "let me know which", "let me know what", "tell me which", "tell me what"];
/// How a part opens when it asks the person for something (17e): an [`ASKS_PERSON`] or
/// [`ASKS_DECISION`] phrase, or one of these. Used for a part that comes before an offer in the same
/// sentence, where an ask word anywhere is too weak: "The upload failed, and I can retry if you'd like"
/// reports a failure.
const ASK_OPENERS: &[&str] = &["please", "i need you to", "you need to", "you'll need to", "can you", "could you"];
/// Words that, right after an opener, make it a name or a report, not an imperative (17e): "Sign in
/// failed with a 401", "Upload is done", "Share link is ready", "Please note…".
const NOT_AFTER_OPENER: &[&str] = &[
    "failed", "fails", "failing", "didn't", "did", "doesn't", "does", "isn't", "is", "was", "wasn't", "won't", "can't", "couldn't",
    "timed", "returned", "worked", "works", "succeeded", "errored", "broke", "stopped", "hung", "needs", "has", "had", "went", "kept",
    "keeps", "step", "steps", "button", "link", "page", "flow", "window", "dialog", "note", "see", "find",
];
/// The timing of an ask, not an offer (16c): "When you're ready, send me the key" still asks for the
/// key. Left out of a part before looking for a wait, and a bare "tell me" next to it is a go-ahead,
/// not an ask ("Whenever you're ready, tell me and I'll deploy").
const WHEN_READY: &[&str] = &["whenever you're ready", "when you're ready"];
/// Where a sentence splits into parts that ask or offer on their own (16c): "Paste the output, and let
/// me know if it fails" asks for the output. `sentence_spans` already splits on "; ".
const PART_BREAKS: &[&str] = &[", or ", ", and ", ", then "];
/// How a trailing alternative of an offer opens (16c): "Let me know when you're done, or if you hit any
/// issues" only offers.
const OFFER_TAILS: &[&str] = &["if you", "when you", "or if you", "or when you"];
/// How much of the ending [`waits_on_person`] keeps.
const WAIT_CHARS: usize = 300;

/// The reply's ending when it waits on the person in prose (13a): its last paragraph has a sentence
/// that waits ("If you approve it, I'll run it", "once you…", "until you…") or asks plainly ("Send me
/// the correct URL", "paste the output", "sign in") or names a pick after ", or " ([`or_names_pick`]),
/// and it isn't a `Done:` or `Blocked:` line or a question (a question is asked on its own path). Returns that paragraph, markdown aside, cut to
/// about [`WAIT_CHARS`] at a word boundary.
pub fn waits_on_person(reply: &str) -> Option<String> {
    if super::keep_going::done_line(reply).is_some() || super::keep_going::blocked_line(reply).is_some() {
        return None;
    }
    let para = reply.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).last()?;
    let bare = para.lines().map(strip_markdown).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ").replace(['’', '‘'], "'");
    let lower = bare.to_ascii_lowercase();
    if lower.trim_end_matches(['*', '_', '`', ')', ' ']).ends_with('?') {
        return None;
    }
    let waits = sentence_spans(&lower).into_iter().any(|(at, end)| {
        let sentence = &lower[at..end];
        asks_in(sentence).into_iter().any(|(part, before)| part_waits(part, before)) || or_names_pick(sentence)
    });
    waits.then(|| cut_at_word(&bare, WAIT_CHARS))
}

/// Whether one part from [`asks_in`] waits on the person: it holds a [`NOT_A_PROMISE`] word, or it
/// asks ([`ASKS_PERSON`], or a decision: [`decides`]) and doesn't open with the bot's own plan. A "when you're
/// ready" in it is the ask's timing: it is left out first, and then a bare "tell me" is a go-ahead, not
/// an ask. A part `before_offer` (it comes before an offer in the same sentence) asks only when it opens
/// like an ask ([`opens_with_ask`]), not with an ask word anywhere in it (17e).
fn part_waits(part: &str, before_offer: bool) -> bool {
    let ready = WHEN_READY.iter().any(|r| part.contains(r));
    let mut s = part.to_string();
    for r in WHEN_READY {
        s = s.replace(r, " ");
    }
    let s = s.trim_start_matches([' ', ',']);
    let planned = PROMISE_OPENERS.iter().any(|o| strip_leads(s).starts_with(o));
    let asks = if before_offer {
        opens_with_ask(s, ready)
    } else {
        (!planned && ASKS_PERSON.iter().filter(|w| !(ready && **w == "tell me")).any(|w| has_words(s, w)))
            || (!ready && decides(s))
    };
    NOT_A_PROMISE.iter().any(|w| s.contains(w)) || asks
}

/// Whether `part` opens like an ask (17e): it, or a clause in it after ", " (leads such as "so" and
/// "then" aside), starts with an [`ASKS_PERSON`], [`ASKS_DECISION`] or [`ASK_OPENERS`] phrase that
/// isn't followed by a [`NOT_AFTER_OPENER`] word. "Please paste the log", "If you can, paste the log",
/// "It failed, so please tell me which mirror to use" open like asks; "The upload failed", "I couldn't
/// connect to the server", "Sign in failed with a 401" and "There is nothing to approve" don't. After
/// "when you're ready" (`ready`), a "tell me…" opener is a go-ahead, as in [`part_waits`].
fn opens_with_ask(part: &str, ready: bool) -> bool {
    part.split(", ").any(|clause| {
        let c = strip_leads(clause.trim_start());
        ASKS_PERSON.iter().chain(ASKS_DECISION).chain(ASK_OPENERS).filter(|o| !(ready && o.starts_with("tell me"))).any(|o| {
            let Some(rest) = c.strip_prefix(o) else { return false };
            if rest.starts_with(|ch: char| ch.is_ascii_alphanumeric()) {
                return false;
            }
            let next = rest.trim_start_matches([' ', ',']).split([' ', ',', '.', ';']).next().unwrap_or("");
            !NOT_AFTER_OPENER.contains(&next) && !(ASKS_DECISION.contains(o) && !decides(c))
        })
    })
}

/// Whether `part` asks the person to decide (19b): it has an [`ASKS_DECISION`] phrase as whole words,
/// and that phrase is "whether to" or names the pick after it ([`picks`]): "Let me know which file to
/// use", "tell me what to do with it", "which one should I use", "which of these two you want", "Let me
/// know which: retry or skip". A closing courtesy after a failed step names no pick and waits on
/// nothing: "Let me know what else you need", "what happens", "what you find", "what you think",
/// "which parts you liked", "which one you'd like to see next". "What you think" never asks, even
/// with an " or " after it ("Let me know what you think or feel").
fn decides(part: &str) -> bool {
    !part.contains("what you think") && ASKS_DECISION.iter().any(|d| {
        part.match_indices(d).any(|(at, m)| {
            let rest = &part[at + m.len()..];
            let whole = !part[..at].ends_with(|c: char| c.is_ascii_alphanumeric()) && !rest.starts_with(|c: char| c.is_ascii_alphanumeric());
            whole && (d.ends_with(" to") || picks(rest))
        })
    })
}

/// Words that, right after "to", make it a place or an object, not a verb (19b): "what happens to the
/// file" isn't "what to do".
const NOT_A_VERB: &[&str] = &[
    "the", "a", "an", "your", "my", "our", "their", "his", "her", "its", "it", "this", "that", "these", "those", "you", "me", "us",
    "them", "him",
];
/// How a "which|what …" names the person's choice (19b): "which option you prefer", "which of these two
/// you want", "what you want me to do". After it, "to" counts only before [`CHOOSE_TO`] ("what you'd
/// like to do", "which way you'd like to proceed", "which one you want to keep") and "me to" only before
/// [`CHOOSE_ME_TO`] ("which option you'd like me to go with"); otherwise it offers more ("which one you'd
/// like to see next", "what you'd like to try next",
/// "which of these you'd like me to expand on", "you'd like to hear more about").
const CHOOSES: &[&str] = &["you want", "you prefer", "you'd like", "you'd prefer", "you would like", "you would prefer"];
/// Verbs after "[`CHOOSES`] to" that hand the person the next step (19b round 2) or name what they pick
/// (21e: "which one you want to keep", "which file you'd like to use"). Not "try": "what you'd like to
/// try next" offers more. Past "do" and "proceed", the verb must end the part or be followed only by
/// "with" (21e round 2): "what you'd like to keep an eye on", "which parts you'd like to go over next"
/// and "what you'd like to take away from this" are closing courtesies.
const CHOOSE_TO: &[&str] = &["do", "proceed", "keep", "use", "pick", "choose", "go", "take", "run"];
/// Verbs after "[`CHOOSES`] me to" that ask the person to pick what the bot does (19b round 2).
const CHOOSE_ME_TO: &[&str] = &["do", "take", "use", "go", "pick", "choose", "keep", "retry", "try", "run"];
/// How `rest` starts when its " or " joins a closer's own words, not options (19b round 2): "what else
/// you need or want", "what happens with the build or the deploy", "which parts worked or didn't".
const OR_IN_CLOSER: &[&str] = &["else", "happens", "you need", "you think", "you find", "parts"];

/// Whether what follows "which" or "what" (`rest`) names a pick (19b): within the wh-word's own word
/// or the one after it (its noun), "to <verb>" ("what to do", "which file to use"), "should" ("which
/// one should I use") or "works" ("what time works"); a [`CHOOSES`] phrase within its first four words,
/// not after "else"; an " or " alternative that isn't an offer's "or if/when" ("which mirror, the US
/// or the EU one") and isn't in a closer ([`OR_IN_CLOSER`]); or a colon that opens the options ("Let me know which: retry or skip", where the
/// sentence ends at the colon).
fn picks(rest: &str) -> bool {
    let words: Vec<&str> = rest.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '\'')).collect();
    let list = rest.trim_end().ends_with(':') && words.len() <= 2;
    let lead = words.iter().take(2).copied().collect::<Vec<_>>().join(" ");
    let closer = OR_IN_CLOSER.iter().any(|c| lead == *c || lead.starts_with(&format!("{c} ")));
    let or = !closer && rest.match_indices(" or ").any(|(at, m)| !["if ", "when "].iter().any(|t| rest[at + m.len()..].starts_with(t)));
    let near = (0..2).any(|i| match words.get(i).copied() {
        Some("to") => words.get(i + 1).is_some_and(|v| !v.is_empty() && !NOT_A_VERB.contains(v)),
        Some("should") | Some("works") => true,
        _ => false,
    });
    let chooses = words.first() != Some(&"else")
        && (0..4).any(|i| {
            CHOOSES.iter().any(|c| {
                let n = c.split(' ').count();
                i + n <= words.len()
                    && words[i..i + n].join(" ") == *c
                    && match words.get(i + n).copied() {
                        Some("to") => words.get(i + n + 1).is_some_and(|v| {
                            // 21e round 2: past "do" and "proceed", a verb names the pick only when the
                            // part ends there or with "with" ("which one you want to keep", "…to go
                            // with"); "what you'd like to keep an eye on" or "to go over next" offers more.
                            let tail = &words[i + n + 2..];
                            CHOOSE_TO.contains(v)
                                && (["do", "proceed"].contains(v)
                                    || tail.iter().all(|w| w.is_empty())
                                    || (tail.first() == Some(&"with") && tail[1..].iter().all(|w| w.is_empty())))
                        }),
                        Some("me") => {
                            words.get(i + n + 1) == Some(&"to") && words.get(i + n + 2).is_some_and(|v| CHOOSE_ME_TO.contains(v))
                        }
                        _ => true,
                    }
            })
        });
    list || or || near || chooses
}

/// The parts of one lowercased `sentence` that may wait on the person: the pieces between its
/// [`PART_BREAKS`] (", or ", ", and ", ", then "), without the ones that hold an offer ([`OFFERS`]),
/// each with whether it comes before an offer in the sentence (17e, see [`part_waits`]). A part that
/// asks for a decision ([`decides`]) isn't an offer: "Let me know whether to retry or skip" asks (17e).
/// The whole part goes, not just the offer's words, so "Let me know once you've merged it" doesn't
/// leave "once you…" behind. A part after a break is its own ask: "Tell me if you want me to run it,
/// or paste the output" still asks for the output, and so does "Paste the output, and let me know if
/// it fails" (16c). A part right after an offer that opens with "if you" or "when you" ([`OFFER_TAILS`])
/// and asks for nothing ([`ASKS_PERSON`]) is the offer's tail and goes with it: "Let me know when
/// you're done, or if you hit any issues."
fn asks_in(sentence: &str) -> Vec<(&str, bool)> {
    let mut parts = Vec::new();
    let mut after_offer = false;
    for part in split_parts(sentence) {
        let part = part.trim_start();
        let offer = !decides(part) && OFFERS.iter().any(|o| has_words(part, o));
        let tail = after_offer
            && OFFER_TAILS.iter().any(|t| part.starts_with(t))
            && !ASKS_PERSON.iter().any(|w| has_words(part, w));
        after_offer = offer || tail;
        parts.push((part, after_offer));
    }
    let last_offer = parts.iter().rposition(|(_, dropped)| *dropped);
    parts
        .into_iter()
        .enumerate()
        .filter(|(_, (_, dropped))| !dropped)
        .map(|(i, (part, _))| (part, last_offer.is_some_and(|l| i < l)))
        .collect()
}

/// How a sentence opens when what follows hands the person the pick (21e), for [`or_names_pick`].
const DECISION_OPENERS: &[&str] = &["let me know", "tell me"];

/// Whether a lowercased `sentence` that opens with a [`DECISION_OPENERS`] phrase names a pick in a part
/// after ", or " (21e): "Let me know what else you need, or which file to use" asks, though its first
/// part is a closer that [`asks_in`] drops as an offer. A part that opens with "which" or "what" is read
/// as "let me know <part>" by [`decides`], so a closer there still names no pick ("…, or what else you
/// need", "…, or what you think"), and a closer alone stays no. Next to "when you're ready" it is a
/// go-ahead, as in [`part_waits`].
fn or_names_pick(sentence: &str) -> bool {
    let s = strip_leads(sentence.trim_start());
    let opens = DECISION_OPENERS.iter().any(|o| s.strip_prefix(o).is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric())));
    opens
        && !WHEN_READY.iter().any(|r| sentence.contains(r))
        && s.split(", or ").skip(1).any(|after| {
            let part = split_parts(after)[0].trim_start();
            (part.starts_with("which ") || part.starts_with("what ")) && decides(&format!("let me know {part}"))
        })
}

/// `sentence` split at each of [`PART_BREAKS`], in order.
fn split_parts(sentence: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = sentence;
    while let Some((at, len)) = PART_BREAKS.iter().filter_map(|b| rest.find(b).map(|at| (at, b.len()))).min() {
        out.push(&rest[..at]);
        rest = &rest[at + len..];
    }
    out.push(rest);
    out
}

/// What went wrong before a turn's own tool steps (13a), for [`failed_step_blocked`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FailedBefore {
    /// A step failed before the turn's own tools: kwnt couldn't open the request's link before the
    /// turn, or the held turn this decided reply replaces ended on a failed step. Counts when the
    /// turn ran no tool after it.
    pub failed: bool,
    /// The person declined an approval card in the held turn this decided reply replaces. Counts
    /// like a card declined in the turn itself.
    pub declined: bool,
}

impl FailedBefore {
    /// What a turn that ends held (see [`super::decide::Held`]) leaves for its decided reply: its
    /// last tool step failed (or its link didn't open and no tool ran after), or it had a declined
    /// card. The decided reply runs as a new turn, so these aren't in its own activity.
    pub fn of_turn(activity: Option<&Value>, before: FailedBefore) -> Self {
        let (ran_tools, failed, declined) = tool_steps_in(activity);
        Self { failed: failed.is_some() || (before.failed && !ran_tools), declined: declined || before.declined }
    }
}

/// What a finished turn is blocked on (13a), for `meta.blocked`: its last tool step failed, the
/// person declined an approval card in it, or a step failed `before` its tools and it ran no tool
/// after that; and its reply ends waiting on the person in prose with no `Done:` or `Blocked:` line
/// ([`waits_on_person`]). `activity` is the turn's saved activity.
pub fn failed_step_blocked(reply: &str, activity: Option<&Value>, before: FailedBefore) -> Option<String> {
    let after = FailedBefore::of_turn(activity, before);
    if !after.failed && !after.declined {
        return None;
    }
    waits_on_person(reply)
}

/// `words` in `text` as whole words (ASCII letters and digits around it don't count).
fn has_words(text: &str, words: &str) -> bool {
    text.match_indices(words).any(|(at, m)| {
        let before = text[..at].chars().next_back();
        let after = text[at + m.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// `text` cut to at most `max` characters at a word boundary, with "…" when cut.
fn cut_at_word(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    let head = match head.rfind(char::is_whitespace) {
        Some(i) if i > 0 => &head[..i],
        _ => &head[..],
    };
    format!("{}…", head.trim_end_matches([',', ';', ':', ' ']))
}

/// A line without its leading list marker, heading or quote, and the emphasis around it.
fn strip_markdown(line: &str) -> String {
    let mut s = line.trim();
    loop {
        let before = s;
        s = s.trim_start_matches(['#', '>', '*', '_', '`', '-', '+', ' ']);
        if let Some((n, rest)) = s.split_once(['.', ')']) {
            if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) {
                s = rest;
            }
        }
        if s == before {
            break;
        }
    }
    s.replace("**", "").replace("__", "").trim().to_string()
}

/// Byte spans of the sentences in `text`: they end at ". ", "! ", "; " or ": ".
fn sentence_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b';' | b':') && bytes.get(i + 1).is_some_and(|n| *n == b' ') {
            out.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push((start, text.len()));
    }
    out
}

/// A sentence without the words a plan may start with ("Next, ", "Now ").
fn strip_leads(mut s: &str) -> &str {
    loop {
        let before = s;
        for lead in PROMISE_LEADS {
            if let Some(rest) = s.strip_prefix(lead) {
                if rest.starts_with([' ', ',']) {
                    s = rest.trim_start_matches([',', ' ']);
                }
            }
        }
        if s == before {
            return s;
        }
    }
}

/// The turn's tool steps, from its saved activity (`{"steps": [...]}`): whether it ran any, its
/// last one when that one failed (a refused tool call is a failed step too), and whether the
/// person declined an approval card in it.
pub fn tool_steps_in(activity: Option<&Value>) -> (bool, Option<FailedStep>, bool) {
    let Some(steps) = activity.and_then(|a| a["steps"].as_array()) else {
        return (false, None, false);
    };
    let declined = steps.iter().any(|s| s["kind"] == "approval" && s["status"] == "declined");
    let last = steps.iter().rfind(|s| s["kind"] == "tool" && !matches!(s["status"].as_str(), Some("running" | "waiting")));
    let failed = last.filter(|s| s["status"] == "failed").map(|s| FailedStep {
        tool: s["tool"].as_str().unwrap_or("a tool").to_string(),
        title: s["title"].as_str().unwrap_or("").to_string(),
        detail: s["detail"].as_str().unwrap_or("").to_string(),
    });
    (last.is_some(), failed, declined)
}

/// The reply links a PR, merge request, branch, compare view or commit on a git host
/// (`…/owner/repo/pull/12`, `…/-/merge_requests/3`, `…/pull-requests/4`, `…/tree/branch`,
/// `…/compare/a...b`, `…/commit/sha`).
pub fn has_change_link(reply: &str) -> bool {
    reply.split(|c: char| c.is_whitespace() || "()<>[]\"'`".contains(c)).any(|word| {
        let Some(rest) = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://")) else {
            return false;
        };
        let segs: Vec<&str> = rest.split(['/', '?', '#']).filter(|s| !s.is_empty()).collect();
        // host, then the path
        let path = segs.get(1..).unwrap_or_default();
        let after = |i: usize| path.get(i + 1).is_some_and(|s| !s.is_empty());
        path.iter().enumerate().any(|(i, s)| match *s {
            // GitHub (and GitHub Enterprise): owner/repo/<kind>/…
            "pull" | "tree" | "compare" | "commit" => i == 2 && after(i),
            // GitLab: group/…/repo/-/<kind>/…
            "merge_requests" | "commits" => i >= 3 && path[i - 1] == "-" && after(i),
            // Bitbucket: workspace/repo/pull-requests/N, or …/branch/name
            "pull-requests" | "branch" => i == 2 && after(i),
            _ => false,
        })
    })
}

/// The turn pushed a commit or opened a PR: a finished tool step that ran `git push`, or a tool
/// whose name says it pushes or opens a pull/merge request. `activity` is the turn's saved
/// activity (`{"steps": [...]}`, see [`crate::drivers::activity::Step`]).
pub fn pushed_in(activity: Option<&Value>) -> bool {
    let Some(steps) = activity.and_then(|a| a["steps"].as_array()) else {
        return false;
    };
    steps.iter().any(|s| {
        if s["kind"] != "tool" || !matches!(s["status"].as_str(), Some("ok" | "done")) {
            return false;
        }
        let tool = s["tool"].as_str().unwrap_or("").to_ascii_lowercase();
        let named = ["push", "create_pull_request", "create_merge_request"].iter().any(|n| tool.contains(n));
        let command = s["detail"].as_str().or_else(|| s["title"].as_str()).unwrap_or("");
        named || runs_git_push(command)
    })
}

/// A shell command that runs `git push` (not a dry run).
fn runs_git_push(command: &str) -> bool {
    let words: Vec<&str> = command.split(|c: char| c.is_whitespace() || "`;&|()".contains(c)).filter(|w| !w.is_empty()).collect();
    words.windows(2).any(|w| w[0] == "git" && w[1] == "push") && !words.iter().any(|w| *w == "--dry-run" || *w == "-n")
}

/// Records whether a turn delivered what its plan expected, as a `check` delivery note (P10,
/// [`super::delivery_plan::record_check`]), kept apart from the person's own notes so the planner
/// never learns from it. Errors are logged there, never returned.
pub async fn record_delivery_note(
    state: &super::AppState,
    kwant_id: &str,
    context_id: &str,
    expect: &Expect,
    outcome: Outcome,
    missing: &[Missing],
) {
    tracing::info!(
        kwant_id,
        context_id,
        image = expect.image,
        link = expect.link,
        firm = expect.firm,
        ?outcome,
        ?missing,
        "delivery check"
    );
    record_in(&state.db, kwant_id, context_id, expect, outcome, missing).await;
}

/// [`record_delivery_note`]'s store step: what was wanted, whether it was all delivered, and what
/// was missing, as [`Show`]s (an image is `Show::Image`, a link `Show::Link`).
async fn record_in(db: &crate::db::Database, kwant_id: &str, context_id: &str, expect: &Expect, outcome: Outcome, missing: &[Missing]) {
    let (wanted, missing) = shows(expect, missing);
    super::delivery_plan::record_check(db, kwant_id, context_id, &wanted, outcome == Outcome::Hit, &missing).await;
}

/// What `expect` wanted and what was `missing`, as the delivery note's [`Show`]s. Follow-through
/// isn't something the plan wanted, so the planner never learns from it.
fn shows(expect: &Expect, missing: &[Missing]) -> (Vec<Show>, Vec<Show>) {
    let show = |m: &Missing| match m {
        Missing::Image => Some(Show::Image),
        Missing::Link => Some(Show::Link),
        Missing::FollowThrough => None,
    };
    let mut wanted = Vec::new();
    if expect.image {
        wanted.push(Show::Image);
    }
    if expect.link {
        wanted.push(Show::Link);
    }
    (wanted, missing.iter().filter_map(show).collect())
}

/// Runs `again` (the continuation turn, given its prompt) when the first turn's check asked for
/// one and the turn wasn't stopped, and joins both replies. `again` is `FnOnce`, so it runs once
/// at most; a failed continuation leaves the first reply as it was.
pub async fn after_turn<F, Fut>(first: Result<String, String>, continue_with: Option<String>, stopped: bool, again: F) -> Result<String, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let (Ok(text), Some(prompt), false) = (&first, continue_with, stopped) else {
        return first;
    };
    match again(prompt).await {
        Ok(more) => Ok(format!("{}\n\n{more}", text.trim_end())),
        Err(_) => first,
    }
}

/// The reply as saved: with [`NO_PICTURE_LINE`](super::picture_claims::NO_PICTURE_LINE) and
/// [`follow_line`] (naming `bot`) when the verdict says so.
pub fn apply(reply: String, v: &Verdict, bot: &str) -> String {
    let reply = if v.no_picture_line { super::picture_claims::with_no_picture_line(&reply) } else { reply };
    match &v.follow_line {
        Some(tool) => format!("{}\n\n{}", reply.trim_end(), follow_line(tool, bot)).trim_start().to_string(),
        None => reply,
    }
}

/// The reply's `meta.delivery`, when the check ran (`checked`) and did something or had an
/// expectation: `{"missing": [...], "auto_shared": bool, "no_picture": bool, "continues": bool,
/// "outcome": "hit"|"miss", "lean": bool, "follow_line": bool}` (`lean`: the plan only leaned, so it didn't continue;
/// `follow_line`: [`follow_line`] was added). A turn the check skips (see `TurnOpts::delivery_checked`) has none.
pub fn meta(v: &Verdict, expect: Option<&Expect>, checked: bool) -> Option<Value> {
    if !checked || (*v == Verdict::default() && expect.is_none()) {
        return None;
    }
    Some(serde_json::json!({
        "missing": v.missing,
        "auto_shared": v.share_last_shot,
        "no_picture": v.no_picture_line,
        "continues": v.continue_with.is_some(),
        "outcome": v.outcome,
        "lean": expect.is_some_and(|e| !e.firm),
        "follow_line": v.follow_line.is_some(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::delivery::Deliverable;
    use serde_json::json;
    use crate::server::picture_claims::NO_PICTURE_LINE;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ideas() -> Expect {
        Expect { image: true, link: false, done_when: vec!["one mockup image per idea is shared".into()], firm: true }
    }
    fn change() -> Expect {
        Expect { image: false, link: true, done_when: vec![], firm: true }
    }
    fn explain() -> Expect {
        Expect::default()
    }
    const NOTHING: Seen = Seen { image_shared: false, has_shot: false, pushed: false, ran_tools: false, failed_step: None, declined: false };
    const SHARED: Seen = Seen { image_shared: true, has_shot: true, ..NOTHING };
    const SHOT: Seen = Seen { has_shot: true, ..NOTHING };

    /// Plays a turn and its continuation the way `routes::try_kwant_turn` does: returns the joined
    /// reply, how many continuations ran, and the last verdict.
    async fn play(expect: Option<Expect>, first: (&str, Seen), then: (&str, Seen)) -> (String, usize, Verdict) {
        let v1 = check(first.0, expect.as_ref(), false, &first.1);
        let runs = AtomicUsize::new(0);
        let last = std::sync::Mutex::new(v1.clone());
        let joined = after_turn(Ok(apply(first.0.into(), &v1, "Ann")), v1.continue_with.clone(), false, |prompt| {
            let (runs, last, expect) = (&runs, &last, expect.clone());
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                assert!(prompt.contains("Missing from your last reply"), "{prompt}");
                let v2 = check(then.0, expect.as_ref(), true, &then.1);
                assert_eq!(v2.continue_with, None, "a continuation never continues again");
                let text = apply(then.0.into(), &v2, "Ann");
                *last.lock().unwrap() = v2;
                Ok(text)
            }
        })
        .await;
        (joined.unwrap(), runs.into_inner(), last.into_inner().unwrap())
    }

    #[tokio::test]
    async fn an_ideas_turn_without_an_image_continues_exactly_once() {
        // The continuation shares nothing either: one continuation, then the plain line, a miss
        let (text, runs, last) = play(Some(ideas()), ("Three ideas for the hero: …", NOTHING), ("Here are the ideas again.", NOTHING)).await;
        assert_eq!(runs, 1);
        assert!(text.starts_with("Three ideas for the hero"), "{text}");
        assert!(!text.starts_with(&format!("Three ideas for the hero: …\n\n{NO_PICTURE_LINE}")), "the first reply waits for the continuation");
        assert!(text.ends_with(NO_PICTURE_LINE), "{text}");
        assert_eq!((last.missing, last.outcome), (vec![Missing::Image], Some(Outcome::Miss)));
        // The continuation shares one: a hit, no line
        let (text, runs, last) = play(Some(ideas()), ("Three ideas: …", NOTHING), ("Shared the mockup.", SHARED)).await;
        assert_eq!(runs, 1);
        assert!(!text.contains(NO_PICTURE_LINE), "{text}");
        assert_eq!(last.outcome, Some(Outcome::Hit));
        // It took a screenshot but didn't share it: kwnt shares that one
        let (_, runs, last) = play(Some(ideas()), ("Three ideas: …", NOTHING), ("Done.", SHOT)).await;
        assert_eq!(runs, 1);
        assert!(last.share_last_shot && last.outcome == Some(Outcome::Hit));
    }

    #[tokio::test]
    async fn the_continuation_prompt_names_what_is_missing() {
        let v = check("Three ideas: …", Some(&ideas()), false, &NOTHING);
        let p = v.continue_with.unwrap();
        assert!(p.contains("Missing from your last reply: an image in the chat."), "{p}");
        assert!(p.contains(IMAGE_PROMPT), "Ada's P9 wording: {p}");
        assert!(p.ends_with("Done when: one mockup image per idea is shared."), "{p}");
        assert!(!p.contains(LINK_PROMPT));
        let both = Expect { image: true, link: true, done_when: vec!["a\nPR   link".into(), "".into()], firm: true };
        let p = check("I changed the header.", Some(&both), false, &NOTHING).continue_with.unwrap();
        assert!(p.contains("an image in the chat and a PR or branch link"), "{p}");
        assert!(p.contains(IMAGE_PROMPT) && p.contains(LINK_PROMPT));
        assert!(p.ends_with("Done when: a PR link."), "one line, blanks dropped: {p}");
    }

    #[tokio::test]
    async fn an_ideas_turn_that_shared_an_image_does_not_continue() {
        let (text, runs, last) = play(Some(ideas()), ("Here is the mockup.", SHARED), ("x", NOTHING)).await;
        assert_eq!((text.as_str(), runs), ("Here is the mockup.", 0));
        assert_eq!(last.outcome, Some(Outcome::Hit));
        // A claim with a screenshot on the first turn is shared, not continued
        let (_, runs, last) = play(Some(ideas()), ("Here is the mockup at phone width.", SHOT), ("x", NOTHING)).await;
        assert_eq!(runs, 0);
        assert!(last.share_last_shot);
    }

    #[tokio::test]
    async fn a_change_or_explain_turn_does_not_continue_for_a_picture() {
        let pushed = Seen { pushed: true, ..NOTHING };
        let link = "Done: https://github.com/alexsergivan/kwant/pull/470";
        for (expect, reply, seen) in [
            (change(), link, NOTHING),
            (change(), "I changed the button colour and pushed it.", pushed),
            (explain(), "The hero uses a grid with two columns.", NOTHING),
            (explain(), "The hero uses a grid with two columns.", SHOT),
        ] {
            let (_, runs, last) = play(Some(expect), (reply, seen), ("x", NOTHING)).await;
            assert_eq!(runs, 0, "{reply}");
            assert!(!last.no_picture_line && !last.share_last_shot, "{reply}");
            assert_eq!(last.outcome, Some(Outcome::Hit), "{reply}");
        }
        // No plan at all (a hand-off, a routine): only the claim guard, never a continuation
        for seen in [NOTHING, SHOT, SHARED] {
            let (_, runs, last) = play(None, ("I changed the button colour.", seen), ("x", NOTHING)).await;
            assert_eq!((runs, last.outcome), (0, None));
        }
    }

    #[tokio::test]
    async fn a_change_without_a_link_continues_once() {
        let (text, runs, last) =
            play(Some(change()), ("I changed the header.", NOTHING), ("Pushed: https://github.com/o/r/tree/vega-x", NOTHING)).await;
        assert_eq!(runs, 1);
        assert!(text.ends_with("https://github.com/o/r/tree/vega-x"), "{text}");
        assert!(!text.contains(NO_PICTURE_LINE), "no picture was asked for");
        assert_eq!(last.outcome, Some(Outcome::Hit));
        let (_, runs, last) = play(Some(change()), ("I changed the header.", NOTHING), ("I can push it if you like.", NOTHING)).await;
        assert_eq!((runs, last.missing, last.outcome), (1, vec![Missing::Link], Some(Outcome::Miss)));
    }

    #[test]
    fn the_claim_guard_works_without_a_plan() {
        let claim = "Here is the mockup at phone width.";
        let v = |seen: Seen| check(claim, None, false, &seen);
        assert!(v(SHOT).share_last_shot && !v(SHOT).no_picture_line);
        assert!(v(NOTHING).no_picture_line && !v(NOTHING).share_last_shot);
        assert_eq!(v(SHARED), Verdict::default());
        assert_eq!(check("The button is orange now.", None, false, &SHOT), Verdict::default());
        assert_eq!(check("I can take a screenshot if you want.", None, false, &NOTHING), Verdict::default());
        assert_eq!(apply(claim.into(), &v(NOTHING), "Ann"), format!("{claim}\n\n{NO_PICTURE_LINE}"));
        assert_eq!(meta(&Verdict::default(), None, true), None);
        assert_eq!(meta(&v(NOTHING), None, true).unwrap()["no_picture"], true);
        // A turn the check skipped records nothing, even with an expectation
        assert_eq!(meta(&Verdict::default(), Some(&ideas()), false), None);
        assert!(meta(&Verdict::default(), Some(&ideas()), true).is_some());
    }

    #[tokio::test]
    async fn a_stopped_or_failed_turn_does_not_continue() {
        let ran = AtomicUsize::new(0);
        let again = |_: String| async {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok::<String, String>("more".into())
        };
        assert_eq!(after_turn(Ok("a".into()), Some("p".into()), true, again).await.unwrap(), "a");
        assert_eq!(after_turn(Err("failed".into()), Some("p".into()), false, |_| async { Ok("more".into()) }).await, Err("failed".into()));
        assert_eq!(after_turn(Ok("a".into()), None, false, |_| async { Ok("more".into()) }).await.unwrap(), "a");
        assert_eq!(ran.into_inner(), 0);
        let r = after_turn(Ok("a".into()), Some("p".into()), false, |_| async { Err::<String, String>("x".into()) }).await;
        assert_eq!(r.unwrap(), "a", "a failed continuation keeps the first reply");
    }

    #[test]
    fn change_links_are_prs_branches_and_commits_only() {
        for yes in [
            "https://github.com/alexsergivan/kwant/pull/470",
            "PR: <https://github.com/o/r/pull/1>.",
            "(https://github.com/o/r/tree/vega-080-share-screens)",
            "https://ghe.example.com/o/r/compare/main...feature",
            "https://github.com/o/r/commit/5ca465e5",
            "https://gitlab.com/g/sub/r/-/merge_requests/3",
            "https://gitlab.com/g/r/-/commits/feature",
            "https://bitbucket.org/w/r/pull-requests/4",
        ] {
            assert!(has_change_link(&format!("Done: {yes}")), "{yes}");
        }
        for no in [
            "I can open a PR once you say go.",
            "https://github.com/o/r",
            "https://github.com/o/r/pulls",
            "https://github.com/o/r/pull/",
            "https://example.com/tree",
            "https://kwant.si/download",
            "github.com/o/r/pull/1",
        ] {
            assert!(!has_change_link(no), "{no}");
        }
    }

    #[test]
    fn a_push_is_a_finished_git_push_or_a_push_tool() {
        let step = |tool: &str, status: &str, detail: &str| serde_json::json!({"steps": [{"kind": "tool", "status": status, "tool": tool, "title": "x", "detail": detail}]});
        assert!(pushed_in(Some(&step("Bash", "ok", "cd site && git push -u origin vega-x"))));
        assert!(pushed_in(Some(&step("mcp__github__push_files", "ok", ""))));
        assert!(pushed_in(Some(&step("mcp__github__create_pull_request", "done", ""))));
        assert!(!pushed_in(Some(&step("Bash", "failed", "git push origin main"))), "a failed push");
        assert!(!pushed_in(Some(&step("Bash", "ok", "git push --dry-run origin x"))));
        assert!(!pushed_in(Some(&step("Bash", "ok", "git status && echo push"))));
        assert!(!pushed_in(Some(&serde_json::json!({"steps": [{"kind": "note", "status": "done", "title": "git push"}]}))));
        assert!(!pushed_in(None));
    }

    fn plan(kind: Deliverable, show: &[Show], done_when: &[&str]) -> DeliveryPlan {
        DeliveryPlan {
            kind,
            show: show.to_vec(),
            who: vec![],
            done_when: done_when.iter().map(|s| s.to_string()).collect(),
            source: crate::router::delivery::PlanSource::Analyzer,
            firm: true,
        }
    }

    /// Oleks's scenario 4 (Lin's visible-change rule): "fix the button" was pushed with no
    /// before/after picture. A Change plan that shows [Link, Image] expects both, and a reply with
    /// only the PR link is missing the image.
    #[test]
    fn a_visible_change_plan_expects_the_link_and_the_picture() {
        let p = plan(
            Deliverable::Change,
            &[Show::Link, Show::Image],
            &["the PR is linked", "a before/after screenshot of the button is shared"],
        );
        let e = Expect::from_plan(&p).expect("a visible change expects something");
        assert!(e.image && e.link, "{e:?}");
        assert_eq!(e.done_when, p.done_when);
        let v = check("Fixed and pushed: https://github.com/alexsergivan/kwant/pull/470", Some(&e), false, &NOTHING);
        assert_eq!(v.missing, vec![Missing::Image], "{v:?}");
        assert_eq!(v.continue_with.as_deref().map(|c| c.contains("before/after screenshot")), Some(true), "{v:?}");
        let pushed = Seen { pushed: true, ..NOTHING };
        assert_eq!(check("Fixed the button.", Some(&e), false, &pushed).missing, vec![Missing::Image]);
        assert_eq!(check("Fixed: https://github.com/o/r/pull/1", Some(&e), false, &SHARED).missing, vec![]);
    }

    #[test]
    fn an_ideas_plan_expects_a_picture_and_a_change_plan_a_link() {
        let p = plan(Deliverable::Ideas, &[Show::Image, Show::Text], &["one mockup image per idea is shared"]);
        let e = Expect::from_plan(&p).expect("Ideas expects a picture");
        assert!(e.image && !e.link, "{e:?}");
        assert_eq!(e.done_when, p.done_when);
        let p = plan(Deliverable::Change, &[Show::Link], &[]);
        let e = Expect::from_plan(&p).expect("a Change expects a link");
        assert!(e.link && !e.image, "{e:?}");
        assert_eq!(check("Done: https://github.com/o/r/pull/1", Some(&e), false, &NOTHING).missing, vec![]);
        assert_eq!(check("Done, it works now.", Some(&e), false, &NOTHING).missing, vec![Missing::Link]);
    }

    #[test]
    fn explain_and_other_plans_expect_nothing() {
        assert_eq!(Expect::from_plan(&plan(Deliverable::Explain, &[Show::Text], &["answers the question"])), None);
        assert_eq!(Expect::from_plan(&plan(Deliverable::Other, &[], &[])), None);
        assert_eq!(Expect::from_plan(&plan(Deliverable::Other, &[Show::Text, Show::Table], &[])), None);
    }

    /// A lean plan ("@Ann look at this", "…propose an architecture refactor") is checked and
    /// recorded, but never continued: no extra turn, and no "no picture" line unless it claims one.
    #[tokio::test]
    async fn a_lean_visual_turn_without_an_image_is_a_miss_that_does_not_continue() {
        let lean = Expect { firm: false, ..ideas() };
        let (text, runs, last) = play(Some(lean.clone()), ("Ann had a look: the layout is fine.", NOTHING), ("x", NOTHING)).await;
        assert_eq!((text.as_str(), runs), ("Ann had a look: the layout is fine.", 0));
        assert_eq!(last.continue_with, None);
        assert!(!last.no_picture_line && !last.share_last_shot, "{last:?}");
        assert_eq!((last.missing, last.outcome), (vec![Missing::Image], Some(Outcome::Miss)));
        assert_eq!(meta(&check("x", Some(&lean), false, &NOTHING), Some(&lean), true).unwrap()["lean"], true);
        // The claims rule is unchanged: a claimed picture still gets the line, or the shot
        let claim = "Here is the mockup at phone width.";
        let v = check(claim, Some(&lean), false, &NOTHING);
        assert!(v.no_picture_line && v.continue_with.is_none(), "{v:?}");
        let v = check(claim, Some(&lean), false, &SHOT);
        assert!(v.share_last_shot && !v.no_picture_line && v.outcome == Some(Outcome::Hit), "{v:?}");
        // A lean turn that shared one is a hit
        assert_eq!(check("Shared it.", Some(&lean), false, &SHARED).outcome, Some(Outcome::Hit));
    }

    #[tokio::test]
    async fn a_lean_change_without_a_link_does_not_continue() {
        let lean = Expect { firm: false, ..change() };
        let (_, runs, last) = play(Some(lean), ("We'll set this up tomorrow.", NOTHING), ("x", NOTHING)).await;
        assert_eq!(runs, 0, "no LINK_PROMPT turn that commits and pushes");
        assert_eq!((last.continue_with, last.no_picture_line), (None, false));
        assert_eq!((last.missing, last.outcome), (vec![Missing::Link], Some(Outcome::Miss)));
    }

    /// Oleks's replay case: "What cool feature can we add?" is a firm Ideas ask, so a reply with
    /// no mockup continues once. The same keyword plan that the hint renders.
    #[tokio::test]
    async fn a_firm_ideas_ask_without_an_image_continues() {
        let plan = crate::router::delivery::keyword_plan("What cool feature can we add?", &[]);
        assert!(plan.firm && plan.kind == Deliverable::Ideas, "{plan:?}");
        let e = Expect::from_plan(&plan).unwrap();
        assert!(e.firm);
        let (_, runs, last) = play(Some(e), ("Three features: …", NOTHING), ("Shared the mockups.", SHARED)).await;
        assert_eq!((runs, last.outcome), (1, Some(Outcome::Hit)));
        // The original wordings plan lean, so they don't continue
        for msg in ["@Ann look at this MENTION:Bob", "Investigate the root cause of this deadlock and propose an architecture refactor"] {
            let plan = crate::router::delivery::keyword_plan(msg, &[]);
            let e = Expect::from_plan(&plan).unwrap_or_else(|| panic!("{msg}: {plan:?}"));
            assert!(!e.firm, "{msg}: {plan:?}");
            assert_eq!(check("Done.", Some(&e), false, &NOTHING).continue_with, None, "{msg}");
        }
    }

    /// P10: the check's verdict lands in the store as one `check` delivery note, filed by what the
    /// plan wanted and naming what was missing, and never among the person's own notes.
    #[tokio::test]
    async fn a_failed_check_writes_one_check_row() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::db::Database::init(tmp.path()).await.unwrap();
        let space = crate::db::spaces::SpaceId::default_space();
        let input = crate::db::models::NewKwant {
            name: "Pixel".into(),
            title: String::new(),
            persona_prompt: String::new(),
            soul: String::new(),
            avatar_sprite: String::new(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
        };
        let bot = db.create_kwant(&space, &input).await.unwrap();
        let v = check("Three features: …", Some(&ideas()), true, &NOTHING);
        assert_eq!((v.outcome, v.missing.clone()), (Some(Outcome::Miss), vec![Missing::Image]));
        record_in(&db, &bot.id, "chat-1", &ideas(), v.outcome.unwrap(), &v.missing).await;
        let rows = db.list_delivery_notes(&space, 10).await.unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let r = &rows[0];
        assert_eq!((r.source.as_str(), r.context_id.as_deref(), r.kwant_id.as_deref()), ("check", Some("chat-1"), Some(bot.id.as_str())));
        assert_eq!((r.kind.as_str(), r.outcome.as_str(), r.wanted.as_deref()), ("visual", "miss", Some("image")));
        assert!(db.list_person_delivery_notes(&space, 10).await.unwrap().is_empty());
        // A change that was pushed is a hit filed as a change
        record_in(&db, &bot.id, "chat-1", &change(), Outcome::Hit, &[]).await;
        let rows = db.list_delivery_notes(&space, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].kind.as_str(), rows[0].outcome.as_str(), rows[0].wanted.as_deref()), ("change", "hit", None));
    }

    #[test]
    fn from_plan_carries_firmness() {
        let lean = DeliveryPlan { firm: false, ..plan(Deliverable::Visual, &[Show::Image], &[]) };
        assert!(!Expect::from_plan(&lean).unwrap().firm);
        assert!(Expect::from_plan(&plan(Deliverable::Visual, &[Show::Image], &[])).unwrap().firm);
    }

    /// 0.8.1 F1 (Rigel's 0.8.0 final QA): a visible change whose first turn shared the screenshot
    /// but gave no PR link. Its continuation only adds the link; counted per turn it asked for the
    /// picture again and ended with the no-picture line. Counted per delivery it doesn't.
    #[test]
    fn a_continuation_counts_the_picture_its_first_turn_shared() {
        let visible = Expect { image: true, link: true, done_when: vec![], firm: true };
        let mut delivery = Seen::default();
        let first = SHARED.with_earlier(&mut delivery);
        let v1 = check("I changed the footer; here is the screenshot.", Some(&visible), false, &first);
        assert_eq!(v1.missing, vec![Missing::Link], "{v1:?}");
        let prompt = v1.continue_with.clone().expect("a missing link continues");
        assert!(prompt.contains("a PR or branch link") && !prompt.contains("an image"), "{prompt}");

        // The continuation shares nothing new and adds the link
        let reply = "Pushed: https://github.com/o/r/pull/12";
        let then = NOTHING.with_earlier(&mut delivery);
        let v2 = check(reply, Some(&visible), true, &then);
        assert!(v2.missing.is_empty() && !v2.no_picture_line && !v2.share_last_shot, "{v2:?}");
        assert_eq!(v2.outcome, Some(Outcome::Hit));
        assert!(!apply(reply.into(), &v2, "Ann").contains(NO_PICTURE_LINE));

        // What the bug did: the continuation's own pictures only
        let per_turn = check(reply, Some(&visible), true, &NOTHING);
        assert_eq!(per_turn.missing, vec![Missing::Image], "{per_turn:?}");
        assert!(per_turn.no_picture_line);
    }

    /// F1: a new ask starts a new delivery, so it never inherits an earlier ask's picture; and a
    /// push one turn made counts for the next, while a screenshot stays the turn's own.
    #[test]
    fn a_new_ask_never_inherits_an_earlier_delivery() {
        let visible = Expect { image: true, link: true, done_when: vec![], firm: true };
        let mut earlier = Seen::default();
        let _ = SHARED.with_earlier(&mut earlier);
        assert!(earlier.image_shared);
        // The next ask: a fresh delivery
        let mut fresh = Seen::default();
        let v = check("I changed the header.", Some(&visible), false, &NOTHING.with_earlier(&mut fresh));
        assert_eq!(v.missing, vec![Missing::Image, Missing::Link], "{v:?}");
        let prompt = v.continue_with.expect("it still asks");
        assert!(prompt.contains("an image in the chat and a PR or branch link"), "{prompt}");

        let mut delivery = Seen::default();
        let pushed = Seen { pushed: true, ..SHOT }.with_earlier(&mut delivery);
        assert_eq!(pushed, Seen { has_shot: true, pushed: true, ..NOTHING });
        let next = NOTHING.with_earlier(&mut delivery);
        assert_eq!(next, Seen { pushed: true, ..NOTHING }, "the shot stays the first turn's");
        assert!(!check("Done, no link here.", Some(&change()), true, &next).missing.contains(&Missing::Link));
    }

    // ---- 0.9.4 follow-through ----

    const REAL: &str = "The render command was refused.\n\nI'll check the file sizes and measure the frame timings next.";

    fn failed(tool: &str) -> Seen {
        Seen { ran_tools: true, failed_step: Some(FailedStep { tool: tool.into(), title: "render the video".into(), detail: String::new() }), ..NOTHING }
    }

    #[test]
    fn a_reply_that_ends_on_a_plan_is_a_promise() {
        assert_eq!(promise_line(REAL).as_deref(), Some("I'll check the file sizes and measure the frame timings next."));
        for (reply, want) in [
            ("Tried it.\n\nLet me retry with a smaller batch.", "Let me retry with a smaller batch."),
            ("Failed.\n\nNext, I’ll open the page in the browser.", "Next, I'll open the page in the browser."),
            ("Failed.\n\n**Now I'll** read the log instead.", "Now I'll read the log instead."),
            ("The build broke. I'm going to run it again with logs.", "I'm going to run it again with logs."),
            ("Summary:\n- tests pass\n- I will push the fix next", "I will push the fix next"),
            ("Refused.\n\nGoing to try the API instead.", "Going to try the API instead."),
        ] {
            assert_eq!(promise_line(reply).as_deref(), Some(want), "{reply}");
        }
    }

    #[test]
    fn done_blocked_questions_and_waits_are_not_promises() {
        for reply in [
            "Done: the render is at out.mp4 and plays.",
            "The render was refused.\n\nBlocked: terminal commands are off for this bot. Allow Terminal, then say continue.",
            "I'll need your pick: should I use the dark theme?",
            "I'll continue once you allow Terminal on the Mac.",
            "I'll pick it up when you've signed in.",
            "Let me know if you want the long version.",
            "I'll wait for your answer.",
            "I'll run it again after you add the key.",
            "If you want, I'll redo it in 4K.",
            "The render failed, so I used the old frames instead. The video is at out.mp4.",
            "I'll check the logs.\n\nThe render is at out.mp4 and plays.",
            "",
        ] {
            assert_eq!(promise_line(reply), None, "{reply}");
        }
    }

    #[test]
    fn the_last_tool_step_is_read_from_the_activity() {
        let steps = |s: Value| Some(json!({ "steps": s }));
        let (ran, f, d) = tool_steps_in(steps(json!([
            { "kind": "thinking", "status": "done" },
            { "kind": "tool", "tool": "Read", "title": "read a.txt", "status": "ok" },
            { "kind": "tool", "tool": "Bash", "title": "render", "status": "failed", "detail": "exit 1: no such file" },
            { "kind": "note", "status": "done" },
        ]))
        .as_ref());
        assert!(ran && !d);
        assert_eq!(f, Some(FailedStep { tool: "Bash".into(), title: "render".into(), detail: "exit 1: no such file".into() }));
        // A later tool step that worked: nothing to follow through
        let (_, f, _) = tool_steps_in(steps(json!([
            { "kind": "tool", "tool": "Bash", "title": "render", "status": "failed" },
            { "kind": "tool", "tool": "Bash", "title": "render small", "status": "ok" },
        ]))
        .as_ref());
        assert_eq!(f, None);
        // A declined card is seen; no activity is nothing
        let (_, _, d) = tool_steps_in(steps(json!([{ "kind": "approval", "status": "declined" }, { "kind": "tool", "tool": "Bash", "status": "failed" }])).as_ref());
        assert!(d);
        assert_eq!(tool_steps_in(None), (false, None, false));
    }

    #[test]
    fn a_failed_step_and_a_promise_continue_once_without_a_plan() {
        let v = check(REAL, None, false, &failed("Bash"));
        assert_eq!(v.missing, vec![Missing::FollowThrough]);
        let prompt = v.continue_with.expect("follow-through continues without a plan");
        assert!(prompt.contains("Your last step (Bash: render the video) failed or was refused"), "{prompt}");
        assert!(prompt.contains("(\"I'll check the file sizes and measure the frame timings next.\")"), "{prompt}");
        assert!(prompt.contains("`Blocked: <what is blocked>. <what would unblock it>`"), "{prompt}");
        assert_eq!(v.outcome, None, "no plan, no delivery note");
        // Lean plans and firm ones alike
        let lean = Expect { firm: false, ..explain() };
        assert!(check(REAL, Some(&lean), false, &failed("Bash")).continue_with.is_some());
        assert!(check(REAL, Some(&explain()), false, &failed("Bash")).continue_with.is_some());
    }

    #[test]
    fn a_later_ok_step_a_declined_card_or_no_promise_doesnt_continue() {
        assert_eq!(check(REAL, None, false, &Seen { ran_tools: true, ..NOTHING }).continue_with, None, "the last step worked");
        assert_eq!(check(REAL, None, false, &Seen { declined: true, ..failed("Bash") }).continue_with, None, "the person declined");
        let blocked = "Refused.\n\nBlocked: terminal is off. Allow Terminal, then say continue.";
        assert_eq!(check(blocked, None, false, &failed("Bash")).continue_with, None);
        assert_eq!(check("The render is at out.mp4.", None, false, &failed("Bash")).continue_with, None);
    }

    #[test]
    fn the_continuation_never_continues_and_gets_the_kwant_line() {
        let v = check("Still failing.\n\nI'll try again later.", None, true, &failed("Bash"));
        assert_eq!(v.continue_with, None);
        assert_eq!(v.follow_line.as_deref(), Some("Bash"));
        assert_eq!(v.stuck_on, failed("Bash").failed_step, "22j: the step the chain is Blocked on");
        let text = apply("Still failing.\n\nI'll try again later.".into(), &v, "Quant");
        assert!(text.ends_with(&follow_line("Bash", "Quant")), "{text}");
        // 22j: the continuation was the other try, so the line says it failed again
        assert_eq!(follow_line("Bash", "Quant"), "_kwant: Bash failed again, and Quant didn't try another way or say why._");
        assert_eq!(meta(&v, None, true).unwrap()["missing"], json!(["follow_through"]));
        assert_eq!(meta(&v, None, true).unwrap()["follow_line"], json!(true));
        // A continuation that got past it, or ended Blocked:, gets no line
        let ok = check("Rendered with ffmpeg instead: out.mp4.", None, true, &Seen { ran_tools: true, ..NOTHING });
        assert_eq!((ok.follow_line, ok.missing), (None, vec![]));
        let b = check("Blocked: terminal is off. Allow it, then say continue.", None, true, &failed("Bash"));
        assert_eq!((b.follow_line, b.stuck_on), (None, None));
    }

    /// 22j: kwant's own Blocked text names the step (a shell step by its command) and the first
    /// line of its error, clipped.
    #[test]
    fn the_blocked_text_names_the_step_and_its_error() {
        let step = |tool: &str, title: &str, detail: &str| FailedStep { tool: tool.into(), title: title.into(), detail: detail.into() };
        let shell = step("run_terminal_command", "Running `journalctl -u maker --since 17:18`", "\nError: no terminal session is open.\nat line 2");
        assert_eq!(shell.name(), "`journalctl -u maker --since 17:18`");
        assert_eq!(shell.error().as_deref(), Some("Error: no terminal session is open"));
        assert_eq!(blocked_text("Quant", &shell), "Quant couldn't run `journalctl -u maker --since 17:18`: Error: no terminal session is open.");
        // No title: the tool's name; no detail: no error
        assert_eq!(blocked_text("Quant", &step("run_terminal_command", "", "")), "Quant couldn't run run_terminal_command.");
        assert_eq!(step("mcp__github__create_issue", "Using create_issue (github)", "401").name(), "create_issue (github)");
        // A long error is cut at a word, at most 160 characters and the ellipsis
        let long = step("Bash", "Running `x`", &"word ".repeat(80)).error().unwrap();
        assert!(long.chars().count() <= 161 && long.ends_with('…'), "{long}");
        assert!(blocked_text("Ann", &step("Bash", "Running `x`", &"word ".repeat(80))).ends_with("word…"));
    }

    #[test]
    fn an_image_miss_and_follow_through_are_one_prompt() {
        let v = check(REAL, Some(&ideas()), false, &failed("Bash"));
        assert_eq!(v.missing, vec![Missing::Image, Missing::FollowThrough]);
        let prompt = v.continue_with.unwrap();
        assert_eq!(prompt.matches("[kwnt: one automatic continuation").count(), 1, "{prompt}");
        assert!(prompt.contains("Missing from your last reply: an image in the chat and another try after the failed step."), "{prompt}");
        assert!(prompt.contains(IMAGE_PROMPT) && prompt.contains("Your last step (Bash"), "{prompt}");
        assert!(prompt.contains("Done when: one mockup image per idea is shared."), "{prompt}");
        // A lean plan's image miss isn't asked for; follow-through still is
        let lean = Expect { firm: false, ..ideas() };
        let prompt = check(REAL, Some(&lean), false, &failed("Bash")).continue_with.unwrap();
        assert!(!prompt.contains(IMAGE_PROMPT) && prompt.contains("Your last step"), "{prompt}");
    }

    #[test]
    fn follow_through_is_never_a_delivery_note() {
        let (_, missing) = shows(&change(), &[Missing::FollowThrough]);
        assert!(missing.is_empty());
        // The outcome after the continuation is the plan's own: a hit here
        let v = check("Still broken.\n\nI'll look again.", Some(&change()), true, &Seen { pushed: true, ..failed("Bash") });
        assert_eq!((v.outcome, v.missing), (Some(Outcome::Hit), vec![Missing::FollowThrough]));
    }

    #[test]
    fn a_continuation_without_tools_keeps_the_failed_step() {
        let mut delivery = Seen::default();
        let first = failed("Bash").with_earlier(&mut delivery);
        assert!(first.failed_step.is_some());
        let then = NOTHING.with_earlier(&mut delivery);
        assert_eq!(then.failed_step, first.failed_step, "it promised again without trying");
        let fixed = Seen { ran_tools: true, ..NOTHING }.with_earlier(&mut delivery);
        assert_eq!(fixed.failed_step, None, "it ran a tool that worked");
        let declined = Seen { declined: true, ..NOTHING }.with_earlier(&mut delivery);
        assert!(declined.declined && Seen::default().with_earlier(&mut delivery).declined);
    }

    #[tokio::test]
    async fn a_failed_step_plays_out_as_one_continuation() {
        let (text, runs, last) = play(None, (REAL, failed("Bash")), ("Rendered with ffmpeg instead: out.mp4.", Seen { ran_tools: true, ..NOTHING })).await;
        assert_eq!(runs, 1);
        assert_eq!(text, format!("{REAL}\n\nRendered with ffmpeg instead: out.mp4."));
        assert_eq!(last.follow_line, None);
    }


    /// The endings of tasks 9 and 10 in Lin's long-task suite (autonomy/long-task-suite/runs,
    /// 093-results.json and 535-results.json): both wait on the person in prose.
    const T9_093: &str = "I didn't run the tests because you denied the approval for the shell command, so I have no result to report.\n\nI would have run `python3 -m unittest` in `/tmp/lts/full/093/run1/t9/work` and reported the pass/fail counts and any failures. If you approve the command, I'll run it right away.";
    const T9_535: &str = "I couldn't run the tests. You denied the approval for the shell command, so `python3 -m unittest` never ran and I have no result to report.\n\nThe command I tried was `python3 -m unittest` from `/tmp/lts/full/535/run1/t9/work`. If you approve it, I'll run it and report the result. You can also run it yourself and paste the output here, and I'll go through any failures with you.";
    const T10_093: &str = "I couldn't download the invoice. `https://billing.example.invalid/invoices` didn't resolve (`net::ERR_NAME_NOT_RESOLVED`), so the page never loaded and nothing was saved to `/tmp/lts/full/093/run1/t10/work/invoice.pdf`.\n\nThe `.invalid` ending is reserved and can never resolve, so this looks like a placeholder address and not your real billing site. I can't guess the real one. Send me the correct URL, for example `https://billing.<yourprovider>.com/invoices`, and I'll open it. If the page needs a login, I'll ask you to sign in in the browser panel, then I'll download the latest invoice and save it as `invoice.pdf`.";
    const T10_535: &str = "I couldn't download the invoice. `https://billing.example.invalid/invoices` didn't resolve (`net::ERR_NAME_NOT_RESOLVED`), so no page loaded and nothing was saved to `invoice.pdf`.\n\nThe `.invalid` ending is a reserved placeholder domain that can never resolve, so it is probably a stand-in and not your real billing address. I can't guess the right one. Look for it in your invoice emails or in your account's billing section. It will usually be something like `billing.<provider>.com/invoices`.\n\nSend me the correct URL and I'll open it. If the site needs a login, I'll ask you to sign in in the browser panel. Then I'll download the latest invoice and save it as `invoice.pdf` in `/tmp/lts/full/535/run1/t10/work`.";
    // Task 9, PR build run 2: the decided reply after the held one (decide first)
    const T9_PR_HELD: &str = "I haven't run the tests, because you declined the approval for `python3 -m unittest` and I didn't ask again. So there is no actual test result.\n\nI haven't edited anything. Running `python3 -m unittest` in that folder would confirm the prediction. Tell me if you want me to run it, or paste the output and I'll check it against the table.";

    #[test]
    fn a_prose_ask_waits_on_the_person() {
        // The real endings: the last paragraph, markdown aside, cut to about 300 characters
        assert_eq!(
            waits_on_person(T9_093).as_deref(),
            Some("I would have run `python3 -m unittest` in `/tmp/lts/full/093/run1/t9/work` and reported the pass/fail counts and any failures. If you approve the command, I'll run it right away.")
        );
        let t9 = waits_on_person(T9_535).unwrap();
        assert!(t9.starts_with("The command I tried was") && t9.contains("If you approve it, I'll run it"), "{t9}");
        for real in [T10_093, T10_535] {
            let w = waits_on_person(real).unwrap();
            assert!(w.chars().count() <= WAIT_CHARS + 1, "{w}");
            assert!(w.contains("Send me the correct URL"), "{w}");
        }
        assert!(waits_on_person(T10_093).unwrap().ends_with('…'), "cut at a word boundary");
        assert!(!waits_on_person(T10_535).unwrap().ends_with('…'), "short enough to keep whole");
        // Plain asks, each as whole words
        for ask in [
            "The page needs a login.\n\nPlease sign in in the browser panel and say continue.",
            "**Upload the CSV** to this chat and I'll combine it.",
            "- Paste the error message here.",
            "Connect your Google account in Settings, then I can read the sheet.",
            "Allow Terminal for this team.",
            "I'll run it as soon as you turn Terminal on.",
            // Task 9 endings of the PR build after a declined card (decided replies, diag runs 2 and 3)
            "The real result is still unconfirmed. If you want it, run `cd work && python3 -m unittest`, or tell me to run it and I'll do so.",
            "If you approve the command, I'll run it and confirm. You can also run `python3 -m unittest` in that folder and paste me the output.",
            // An offer next to a real ask: the ask after ", or" still waits (T9_PR_HELD's ending)
            "Tell me if you want me to run it, or paste the output and I'll check it against the table.",
            // Waits and asks that keep counting next to the offers below
            "I will try again once you approve the next card.",
            "Tell me to run it and I'll report the counts.",
        ] {
            assert!(waits_on_person(ask).is_some(), "{ask}");
        }
        assert!(waits_on_person(T9_PR_HELD).is_some());
        // A promise ("I'll share…" is the bot's own plan), a Done: or Blocked: line, a question,
        // an offer, or a word that only contains an ask ("connection", "shared", "allowed")
        for not in [
            "The render failed.\n\nI'll retry the render with a smaller size next.",
            "The build failed. Next I'll share the logs with Lin.",
            "Fixed it.\n\nDone: the tests pass.",
            "Blocked: no key for the Stripe API. Add it in Settings.",
            "I can't reach the site. Should I try the mirror instead?",
            "Here's the summary.\n\nLet me know if you want changes.",
            "If you'd like, I can also add a chart.",
            // Offers with another opener (Rigel's S1/S2): the whole sentence goes, not just "let me know"
            "The draft is in the folder.\n\nLet me know when you've had a chance to review it.",
            "The draft is in the folder.\n\nLet me know once you've had a chance to review it.",
            "The notes are drafted.\n\nLet me know once you've merged it and I'll update the notes.",
            "The build is ready.\n\nWhenever you're ready, tell me and I'll deploy.",
            "The download failed.\n\nTell me if you want me to try a different mirror.",
            "The download failed.\n\nTell me whether you want me to try a different mirror.",
            "When you're ready, tell me and I'll ship it.",
            "The connection failed, so I used the cached copy. The report is shared in the folder and the tests are allowed to run.",
            "",
        ] {
            assert_eq!(waits_on_person(not), None, "{not}");
        }
    }

    /// Rigel's F1–F3 from the QA of #639 (autonomy/failed-step-offers/qa.md), and #639's own offers.
    #[test]
    fn an_ask_next_to_an_offer_still_waits_and_an_offers_tail_does_not() {
        let table: &[(&str, bool)] = &[
            // F1: an ask that shares a sentence with an offer, or with "when you're ready", still waits
            ("Paste the output, and let me know if it fails.", true),
            ("Please sign in, and tell me if you want me to retry.", true),
            ("Connect your account, then tell me if it worked.", true),
            ("Whenever you're ready, please paste the output.", true),
            ("When you're ready, send me the key.", true),
            ("Approve the card when you're ready and I'll run it.", true),
            // F2: "tell me if/whether" without "you" asks for a decision or a fact
            ("Tell me whether to retry or skip.", true),
            ("Tell me if the key is expired.", true),
            // An alternative after an offer that asks for something still waits
            ("Tell me if you want me to run it, or paste the output and I'll check it against the table.", true),
            ("Let me know if it works, or if you can, paste the log.", true),
            // F3: an offer's trailing alternative only offers
            ("Let me know when you're done, or if you hit any issues.", false),
            ("Let me know if you want me to retry, or if you'd rather skip it.", false),
            ("Tell me if you want me to try a different mirror, or if you'd rather wait.", false),
            // #639's offers still give none
            ("Let me know when you've had a chance to review it.", false),
            ("Let me know once you've merged it and I'll update the notes.", false),
            ("Whenever you're ready, tell me and I'll deploy.", false),
            ("When you're ready, tell me and I'll ship it.", false),
            ("Open it when you're ready and tell me what to change.", false),
            ("Tell me if you want me to try a different mirror.", false),
            ("Tell me whether you want me to try a different mirror.", false),
            ("If you'd like, I can also add a chart.", false),
            ("I'll deploy it whenever you're ready.", false),
        ];
        for (sentence, waits) in table {
            let reply = format!("The download failed.\n\n{sentence}");
            assert_eq!(waits_on_person(&reply).is_some(), *waits, "{sentence}");
        }
    }

    /// Rigel's G and F4 from the QA of #658 (autonomy/blocked-asks-with-offers/qa.md), 17e.
    #[test]
    fn a_failure_report_before_an_offer_is_not_an_ask_and_a_decision_is() {
        let table: &[(&str, bool)] = &[
            // G: a part before an offer that reports a failure or names a thing doesn't ask (G1-G5, G7-G9, C4, C5)
            ("The upload failed, and I can retry if you'd like.", false),
            ("The upload failed because the file is too large, and let me know if you want me to compress it.", false),
            ("I couldn't connect to the server, and I can try again if you'd like.", false),
            ("Sign in failed with a 401, and let me know if you want me to retry.", false),
            ("I couldn't paste it into the doc, and I can try the clipboard route if you'd like.", false),
            ("There is nothing to approve, and let me know if you'd like anything else.", false),
            ("Here is the share link, and let me know if you'd like a shorter one.", false),
            ("The log in the file is empty, and let me know if you want me to rerun it.", false),
            ("I couldn't connect to the API, and let me know if you want me to retry.", false),
            ("I couldn't share the file, and let me know if you want me to try again.", false),
            ("Upload failed, then let me know if you want a retry.", false),
            ("Please note the upload failed, and let me know if you want a retry.", false),
            // ...but one that opens like an ask, at its start or after a comma, still asks
            ("Please paste the log, and let me know if you want a retry.", true),
            ("If you can, paste the log, and let me know if you want a retry.", true),
            ("It failed, so please tell me which mirror to use, or tell me if you'd rather skip it.", true),
            ("I need you to approve the command, and let me know when it's done.", true),
            ("Sign in to the browser panel, and let me know if it fails.", true),
            ("Share the new link and I'll download it, and let me know if you'd rather skip.", true),
            ("When you're ready, send me the key, and let me know if it fails.", true),
            ("Tell me which one to retry, or let me know if you want to skip both.", true),
            // F4: a decision is an ask, not an offer
            ("Let me know whether to retry or skip.", true),
            ("Let me know whether to retry or skip it.", true),
            ("Let me know which file to use.", true),
            ("Let me know what to do with the old copy.", true),
            ("Let me know whether to retry, or if you'd rather skip it.", true),
            ("Tell me which mirror to use.", true),
            // ...and these stay offers
            ("Let me know how it goes.", false),
            ("Let me know if you want me to retry or skip it.", false),
            ("Let me know whether you'd like a retry.", false),
            ("Let me know what you think.", false),
            ("Open it when you're ready and tell me what to change.", false),
        ];
        for (sentence, waits) in table {
            let reply = format!("The download failed.\n\n{sentence}");
            assert_eq!(waits_on_person(&reply).is_some(), *waits, "{sentence}");
        }
    }

    #[test]
    fn a_closing_courtesy_is_not_a_decision_and_a_named_pick_is() {
        let table: &[(&str, bool)] = &[
            // 19b: closers after a failed step name no pick, so no Blocked (Rigel's F1: R1-R4, R6, R10, R32, R42)
            ("Let me know what else I can help with.", false),
            ("Let me know what else you need.", false),
            ("Let me know what happens.", false),
            ("Let me know what you find.", false),
            ("Let me know which of these you'd like me to expand on.", false),
            ("I fixed it, and let me know which parts you liked.", false),
            ("Let me know what went wrong on your side, if anything.", false),
            ("Done. Let me know which one you'd like to see next.", false),
            ("Let me know what happens to the file.", false),
            ("Let me know what else you want.", false),
            ("Let me know what you need to proceed.", false),
            ("Let me know what happens, or if it fails again.", false),
            ("Let me know what you find or if anything else breaks.", false),
            // F4 and other named picks still say Blocked
            ("Let me know whether to retry or skip.", true),
            ("Let me know whether to retry or skip it.", true),
            ("Let me know which file to use.", true),
            ("Let me know what to do with the broken file.", true),
            ("Let me know whether to retry, or if you'd rather skip it.", true),
            ("Tell me which mirror to use.", true),
            ("Let me know which one should I use.", true),
            ("Let me know which: retry or skip.", true),
            ("Let me know which you'd prefer, the US or the EU mirror.", true),
            ("Let me know which of these two you want.", true),
            ("Let me know which option you prefer, and I'll retry.", true),
            ("Let me know what you want me to do.", true),
            ("Let me know what time works and I'll schedule it.", true),
            // Round 2 (Rigel's qa-runs/mine.txt): "you'd like (me) to <next step>" is a pick (D1, D4, D17) ...
            ("Let me know what you'd like to do.", true),
            ("Let me know which approach you'd like me to take.", true),
            ("Let me know which way you'd like to proceed.", true),
            ("Let me know which one you'd like me to retry.", true),
            ("Let me know what you want me to try next.", true),
            ("Let me know which ones you'd like to hear more about.", false),
            // ... "what you think" never asks (C2), and an " or " inside a closer isn't options (C1, C4)
            ("Let me know what you think or feel.", false),
            ("Let me know what else you need or want.", false),
            ("Let me know which parts worked or didn't.", false),
            ("Let me know what happens with the build or the deploy.", false),
            // 21e (Rigel's #679 QA): a plain pick after a closer, and "you want to keep", say Blocked ...
            ("Let me know what else you need, or which file to use.", true),
            ("Let me know which one you want to keep.", true),
            ("Tell me what went wrong on your side, or which mirror to use.", true),
            ("Let me know which file you'd like to use.", true),
            ("Let me know which way you want to go.", true),
            ("Let me know which version you want to go with.", true),
            // ... the three arguable ones stay no, and so does a closer after ", or "
            ("Let me know what you need or want me to do.", false),
            ("Let me know what you'd like to try next.", false),
            ("Let me know which parts you think need work or which you'd like me to redo.", false),
            ("Let me know what happens, or what else you need.", false),
            ("Let me know how it goes, or what you think.", false),
            ("Let me know what else you need, or which ones you'd like to see next.", false),
            ("When you're ready, let me know what else you need, or which file to use.", false),
            ("I can retry, or which file to use.", false),
            // ... and so does a new verb with more words after it (21e round 2, Rigel's #699 QA)
            ("Let me know what you'd like to keep an eye on.", false),
            ("Let me know which parts you'd like to go over next.", false),
            ("Let me know what else you need, or what you'd like to take away from this.", false),
        ];
        for (sentence, waits) in table {
            let reply = format!("The download failed.\n\n{sentence}");
            assert_eq!(waits_on_person(&reply).is_some(), *waits, "{sentence}");
        }
        // The shape on its own: what follows "which" or "what"
        assert!(picks(" file to use") && picks(" to do") && picks(" one should i use") && picks(":"));
        assert!(!picks(" happens to the file") && !picks(" else you want") && !picks(" happens or if it fails") && !picks(""));
        assert!(picks(" you'd like to do") && picks(" option you'd like me to go with") && picks(" mirror, the us or the eu one"));
        assert!(!picks(" you'd like to see next") && !picks(" you'd like me to expand on") && !picks(" parts worked or didn't"));
        assert!(decides("let me know whether to retry") && !decides("let me know whatever you need") && !decides("tell me what you think"));
        // 21e: the part after ", or " is read on its own, only after an opener
        assert!(or_names_pick("let me know what else you need, or which file to use.") && or_names_pick("so tell me what you saw, or which one to keep"));
        assert!(!or_names_pick("let me know what else you need.") && !or_names_pick("i can retry, or which file to use"));
        assert!(!or_names_pick("let me know what happens, or what else you need") && !or_names_pick("let me know, or whatever you need"));
        assert!(picks(" one you want to keep") && !picks(" you'd like to try next"));
        assert!(picks(" branch you want to go with") && picks(" you'd like to do next") && !picks(" you'd like to keep an eye on"));
    }

    #[test]
    fn parts_split_at_or_and_then() {
        assert_eq!(split_parts("paste it, and let me know, then go, or wait"), ["paste it", "let me know", "go", "wait"]);
        assert_eq!(split_parts("bread and butter, salt"), ["bread and butter, salt"]);
        assert_eq!(asks_in("let me know if it fails, or if you can't, and send me the log"), [("send me the log", false)]);
        // A part before an offer is marked, one after it isn't (17e)
        assert_eq!(asks_in("the upload failed, and i can retry if you'd like, or paste the log"), [("the upload failed", true), ("paste the log", false)]);
        assert_eq!(asks_in("let me know whether to retry, or if you'd rather skip it"), [("let me know whether to retry", false), ("if you'd rather skip it", false)]);
    }

    #[test]
    fn a_failed_or_declined_step_and_a_prose_ask_are_blocked() {
        let steps = |s: Value| Some(json!({ "steps": s }));
        let none = FailedBefore::default();
        let page = FailedBefore { failed: true, declined: false };
        let failed = steps(json!([{ "kind": "tool", "tool": "Bash", "status": "failed", "title": "python3 -m unittest" }]));
        let declined = steps(json!([{ "kind": "approval", "status": "declined" }, { "kind": "tool", "tool": "Read", "status": "ok" }]));
        let fine = steps(json!([{ "kind": "tool", "tool": "Bash", "status": "failed" }, { "kind": "tool", "tool": "Bash", "status": "ok" }]));
        // The last tool step failed, or the person declined a card: the ending is blocked
        assert_eq!(failed_step_blocked(T9_093, failed.as_ref(), none).as_deref(), waits_on_person(T9_093).as_deref());
        assert!(failed_step_blocked(T9_535, declined.as_ref(), none).is_some());
        // The request's link didn't open before the turn (task 10) and no tool ran after it
        assert_eq!(failed_step_blocked(T10_535, None, page).as_deref(), waits_on_person(T10_535).as_deref());
        assert!(failed_step_blocked(T10_093, steps(json!([])).as_ref(), page).is_some());
        // ...but a tool that ran fine after it (the bot found the page another way) clears it
        let ok = steps(json!([{ "kind": "tool", "tool": "WebSearch", "status": "ok" }]));
        assert_eq!(failed_step_blocked(T10_535, ok.as_ref(), page), None);
        // No failed step (an earlier one recovered), or no tools and the link opened: nothing to store
        assert_eq!(failed_step_blocked(T9_093, fine.as_ref(), none), None);
        assert_eq!(failed_step_blocked(T10_535, None, none), None);
        // A failed step and an ending that doesn't wait (a promise, an explicit Blocked: line,
        // which the save step keeps in its own words): none from here
        assert_eq!(failed_step_blocked("It failed.\n\nI'll retry it next.", failed.as_ref(), none), None);
        assert_eq!(failed_step_blocked("It failed.\nBlocked: Terminal is off. Allow it, then say continue.", failed.as_ref(), none), None);
        // A held reply's decided turn (decide first) runs as a new turn: what the held turn's steps
        // left counts as if it were the decided turn's own. Task 9, 2nd run of the PR build: the
        // card was declined in the held turn, the decided turn only read the files
        let held = steps(json!([{ "kind": "approval", "status": "declined" }, { "kind": "tool", "tool": "Bash", "status": "failed" }]));
        let left = FailedBefore::of_turn(held.as_ref(), none);
        assert_eq!(left, FailedBefore { failed: true, declined: true });
        let reads = steps(json!([{ "kind": "tool", "tool": "Bash", "status": "ok" }, { "kind": "tool", "tool": "Read", "status": "ok" }]));
        assert_eq!(failed_step_blocked(T9_PR_HELD, reads.as_ref(), left).as_deref(), waits_on_person(T9_PR_HELD).as_deref());
        assert!(waits_on_person(T9_PR_HELD).is_some_and(|t| t.contains("paste the output")));
        assert_eq!(failed_step_blocked(T9_PR_HELD, reads.as_ref(), none), None, "nothing failed in its own turn");
        // A held turn's failed step (no card) counts only while the decided turn runs no tool
        let failed_only = FailedBefore::of_turn(failed.as_ref(), none);
        assert_eq!(failed_only, FailedBefore { failed: true, declined: false });
        assert!(failed_step_blocked(T9_093, None, failed_only).is_some());
        assert_eq!(failed_step_blocked(T9_093, reads.as_ref(), failed_only), None);
        // ...and a link that didn't open carries over the same way, unless a tool ran after it
        assert_eq!(FailedBefore::of_turn(None, page), page);
        assert_eq!(FailedBefore::of_turn(reads.as_ref(), page), none);
    }

    /// 28b: a tool step the person said no to counts as declined, with or without a card (#781's
    /// no-card refusal has none), and isn't a failed step to follow through.
    #[test]
    fn a_declined_tool_step_counts_as_declined() {
        let activity = json!({"steps": [
            {"kind": "tool", "tool": "Bash", "title": "Running `rm -rf build`", "status": "declined", "detail": "You said no; nothing ran"},
        ]});
        let (_, failed, declined) = tool_steps_in(Some(&activity));
        assert!(declined);
        assert_eq!(failed, None);
    }
}
