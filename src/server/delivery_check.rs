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
//!   on a promise after a failed step, its reply gets [`follow_line`]. A step the person declined
//!   on an approval card is never pushed again. Follow-through is never a delivery note.
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

/// A tool step that failed or was refused: the tool's name and the step's short title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedStep {
    pub tool: String,
    pub title: String,
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

/// Added to a continuation's reply that still ends on a promise after a failed step.
pub fn follow_line(tool: &str) -> String {
    format!("_kwant: this stopped after a failed step ({tool}) without another try or a reason._")
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
        v.follow_line = follow.map(|(step, _)| step.tool.clone());
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
/// [`follow_line`] when the verdict says so.
pub fn apply(reply: String, v: &Verdict) -> String {
    let reply = if v.no_picture_line { super::picture_claims::with_no_picture_line(&reply) } else { reply };
    match &v.follow_line {
        Some(tool) => format!("{}\n\n{}", reply.trim_end(), follow_line(tool)).trim_start().to_string(),
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
        let joined = after_turn(Ok(apply(first.0.into(), &v1)), v1.continue_with.clone(), false, |prompt| {
            let (runs, last, expect) = (&runs, &last, expect.clone());
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                assert!(prompt.contains("Missing from your last reply"), "{prompt}");
                let v2 = check(then.0, expect.as_ref(), true, &then.1);
                assert_eq!(v2.continue_with, None, "a continuation never continues again");
                let text = apply(then.0.into(), &v2);
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
        assert_eq!(apply(claim.into(), &v(NOTHING)), format!("{claim}\n\n{NO_PICTURE_LINE}"));
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
        assert!(!apply(reply.into(), &v2).contains(NO_PICTURE_LINE));

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
        Seen { ran_tools: true, failed_step: Some(FailedStep { tool: tool.into(), title: "render the video".into() }), ..NOTHING }
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
            { "kind": "tool", "tool": "Bash", "title": "render", "status": "failed" },
            { "kind": "note", "status": "done" },
        ]))
        .as_ref());
        assert!(ran && !d);
        assert_eq!(f, Some(FailedStep { tool: "Bash".into(), title: "render".into() }));
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
        let text = apply("Still failing.\n\nI'll try again later.".into(), &v);
        assert!(text.ends_with(&follow_line("Bash")), "{text}");
        assert_eq!(follow_line("Bash"), "_kwant: this stopped after a failed step (Bash) without another try or a reason._");
        assert_eq!(meta(&v, None, true).unwrap()["missing"], json!(["follow_through"]));
        assert_eq!(meta(&v, None, true).unwrap()["follow_line"], json!(true));
        // A continuation that got past it, or ended Blocked:, gets no line
        let ok = check("Rendered with ffmpeg instead: out.mp4.", None, true, &Seen { ran_tools: true, ..NOTHING });
        assert_eq!((ok.follow_line, ok.missing), (None, vec![]));
        let b = check("Blocked: terminal is off. Allow it, then say continue.", None, true, &failed("Bash"));
        assert_eq!(b.follow_line, None);
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
}
