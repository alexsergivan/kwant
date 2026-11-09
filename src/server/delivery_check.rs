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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    /// It put an image into the chat (`share_file`, `browser_screenshot` with `share: true`).
    pub image_shared: bool,
    /// It took a screenshot (the one the guard can share).
    pub has_shot: bool,
    /// It pushed a commit or opened a PR (see [`pushed_in`]).
    pub pushed: bool,
}

/// Something the person expected that the turn didn't deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// No image in the chat.
    Image,
    /// No PR or branch link in the reply, and nothing pushed.
    Link,
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
}

/// The P9 continuation for a missing image (Ada's wording).
pub const IMAGE_PROMPT: &str = "You were asked for something to see. Render it (use the project's own CSS for a mockup), \
take a screenshot with share: true, and reply.";

/// The P10 continuation for a missing link.
pub const LINK_PROMPT: &str = "You were asked for a change, and your reply has no link to it. Commit and push your work, \
then reply with the PR or branch link.";

/// Most `done_when` checks quoted in a continuation, and the longest one (characters).
const MAX_CHECKS: usize = 5;
const MAX_CHECK_CHARS: usize = 200;

/// Checks one reply. `continued` says the turn is the one continuation already. A lean `expect`
/// never continues and adds the no-picture line only for a claimed picture.
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
    let firm = expect.is_some_and(|e| e.firm);
    match expect {
        Some(e) if firm && !v.missing.is_empty() && !continued => {
            v.continue_with = Some(continuation_prompt(&v.missing, &e.done_when))
        }
        _ => {
            v.no_picture_line = !image && (claims || (wants_image && firm));
            v.outcome = expect.map(|_| if v.missing.is_empty() { Outcome::Hit } else { Outcome::Miss });
        }
    }
    v
}

/// The continuation's prompt: what is missing, how to deliver it, and the plan's checks.
pub fn continuation_prompt(missing: &[Missing], done_when: &[String]) -> String {
    let what: Vec<&str> = missing
        .iter()
        .map(|m| match m {
            Missing::Image => "an image in the chat",
            Missing::Link => "a PR or branch link",
        })
        .collect();
    let mut out = format!("[kwnt: one automatic continuation. Missing from your last reply: {}.]", what.join(" and "));
    for m in missing {
        out.push(' ');
        out.push_str(match m {
            Missing::Image => IMAGE_PROMPT,
            Missing::Link => LINK_PROMPT,
        });
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

/// What `expect` wanted and what was `missing`, as the delivery note's [`Show`]s.
fn shows(expect: &Expect, missing: &[Missing]) -> (Vec<Show>, Vec<Show>) {
    let show = |m: &Missing| match m {
        Missing::Image => Show::Image,
        Missing::Link => Show::Link,
    };
    let mut wanted = Vec::new();
    if expect.image {
        wanted.push(Show::Image);
    }
    if expect.link {
        wanted.push(Show::Link);
    }
    (wanted, missing.iter().map(show).collect())
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

/// The reply as saved: with [`NO_PICTURE_LINE`](super::picture_claims::NO_PICTURE_LINE) when the verdict says so.
pub fn apply(reply: String, v: &Verdict) -> String {
    if v.no_picture_line {
        super::picture_claims::with_no_picture_line(&reply)
    } else {
        reply
    }
}

/// The reply's `meta.delivery`, when the check ran (`checked`) and did something or had an
/// expectation: `{"missing": [...], "auto_shared": bool, "no_picture": bool, "continues": bool,
/// "outcome": "hit"|"miss", "lean": bool}` (`lean`: the plan only leaned, so it didn't continue). A turn the check skips (see `TurnOpts::delivery_checked`) has none.
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
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::delivery::Deliverable;
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
    const NOTHING: Seen = Seen { image_shared: false, has_shot: false, pushed: false };
    const SHARED: Seen = Seen { image_shared: true, has_shot: true, pushed: false };
    const SHOT: Seen = Seen { image_shared: false, has_shot: true, pushed: false };

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
}
