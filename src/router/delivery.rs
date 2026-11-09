//! How a turn should deliver its result (P9/P10): the kind of result the person's message expects
//! (ideas to choose from, something to see, a change made, an answer), what they should see in
//! the chat, and the plain checks that say it was delivered. [`keyword_plan`] makes one from the
//! message's words, like the role signals in [`super::team`], with no model call, so every turn
//! has a plan at once. An analyzer or learned notes can make a better [`DeliveryPlan`] later; the
//! prompt's "[What to deliver]" hint renders from whichever plan the turn carries
//! ([`DeliveryPlan::hint`]), and the delivery check after the turn reads the same plan from
//! [`crate::drivers::TurnInput::delivery`].

use serde::{Deserialize, Serialize};

/// The kind of result a message expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Deliverable {
    /// "What cool feature can we add?": a few grounded ideas, each with a mockup, and one pick.
    Ideas,
    /// "Show me", "a mockup", "a screenshot": an image shared in the chat.
    Visual,
    /// "Fix …", "add …": the change made on a branch, checked, with a PR.
    Change,
    /// A question: a direct answer grounded in the project.
    Explain,
    /// A greeting, a thank-you or a go-ahead: nothing in particular to deliver.
    Other,
}

/// What the person should see in the chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Show {
    Image,
    Link,
    Table,
    File,
    Video,
    Text,
}

impl Show {
    fn label(self) -> &'static str {
        match self {
            Self::Image => "an image",
            Self::Link => "a link (the PR or page)",
            Self::Table => "a table",
            Self::File => "a file",
            Self::Video => "a video",
            Self::Text => "a short text",
        }
    }
}

/// Where a plan came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanSource {
    /// [`keyword_plan`]: the message's words.
    Keywords,
    /// A model call that read the message, the chat and the team.
    Analyzer,
    /// The person's own delivery notes.
    Learned,
}

/// How a turn should deliver its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPlan {
    pub kind: Deliverable,
    /// What the person should see in the chat, e.g. Ideas → [Image, Text].
    pub show: Vec<Show>,
    /// Team chats: (bot id, its part); the first one answers the person. Empty for the keyword plan.
    #[serde(default)]
    pub who: Vec<(String, String)>,
    /// Plain checks, e.g. "one mockup image per idea is shared in the chat".
    #[serde(default)]
    pub done_when: Vec<String>,
    pub source: PlanSource,
}

impl DeliveryPlan {
    /// The default plan for `kind`: what to show and the checks for it.
    pub fn for_kind(kind: Deliverable, source: PlanSource) -> Self {
        let (show, done_when): (Vec<Show>, &[&str]) = match kind {
            Deliverable::Ideas => (vec![Show::Image, Show::Text], &["one mockup image per idea is shared in the chat"]),
            Deliverable::Visual => (vec![Show::Image], &["the image is shared in the chat"]),
            Deliverable::Change => {
                (vec![Show::Link], &["the PR link is in the reply", "a screenshot is shared if the UI changed"])
            }
            Deliverable::Explain | Deliverable::Other => (vec![Show::Text], &[]),
        };
        Self { kind, show, who: Vec::new(), done_when: done_when.iter().map(|s| s.to_string()).collect(), source }
    }

    /// Whether the person should get an image in the chat.
    pub fn wants_picture(&self) -> bool {
        self.show.contains(&Show::Image)
    }

    /// The one-line "[What to deliver]" prompt hint, starting with a blank line; empty for
    /// [`Deliverable::Other`].
    pub fn hint(&self) -> String {
        let what = match self.kind {
            Deliverable::Ideas => "Ideas: read the project first, then give 2–3 ideas grounded in it, each with why, what it \
                would change and a rough effort, and a mockup of each rendered in the browser with the project's real CSS, \
                screenshotted and shared in the chat. End with one recommendation and one choice card for which to build.",
            Deliverable::Visual => "Something to see: render it and share the image in the chat, every time; describing it isn't enough.",
            Deliverable::Change => "A change: make it on a branch, run the checks, open the PR and share a screenshot if the UI \
                changed, then report what you did and what you checked.",
            Deliverable::Explain => "An answer: answer directly, grounded in the project's files or site.",
            Deliverable::Other => return String::new(),
        };
        let mut line = format!("\n\n[What to deliver] {what}");
        if !self.show.is_empty() {
            let show: Vec<&str> = self.show.iter().map(|s| s.label()).collect();
            line.push_str(&format!(" Show in the chat: {}.", show.join(", ")));
        }
        if !self.done_when.is_empty() {
            line.push_str(&format!(" Done when: {}.", self.done_when.join("; ")));
        }
        line
    }
}

/// Words that ask for ideas on their own.
const IDEA_WORDS: &[&str] = &[
    "idea", "ideas", "suggest", "suggestion", "suggestions", "improve", "improvement", "improvements", "brainstorm",
    "propose", "proposal", "proposals", "inspiration", "roadmap",
];
/// "feature" with one of these asks for new ones ("a cool feature we can add", "new features").
const FEATURE_WORDS: &[&str] = &["feature", "features"];
const FEATURE_ASK: &[&str] = &["new", "add", "cool", "could", "can", "should", "next", "other", "more", "else"];
/// Phrases that ask for ideas.
const IDEA_PHRASES: &[&str] = &[
    "what could we add", "what can we add", "what should we add", "what else could", "what else can",
    "what would you add", "what would make", "how could we make", "how can we make",
];
/// Words that ask to see something.
const SEE_WORDS: &[&str] = &[
    "show", "see", "mockup", "mockups", "mock-up", "mock-ups", "screenshot", "screenshots", "image", "images",
    "picture", "pictures", "pic", "preview", "visualize", "visualise",
];
/// Words that only lean visual: "fix the design" is a change, "the design?" is something to see.
const LOOK_WORDS: &[&str] = &["look", "looks", "design", "designs", "visual", "visually", "appearance"];
/// Verbs that ask for a change.
const CHANGE_VERBS: &[&str] = &[
    "fix", "add", "make", "build", "change", "update", "implement", "create", "remove", "delete", "rename",
    "refactor", "deploy", "push", "commit", "ship", "write", "edit", "redo", "merge", "move", "replace", "install",
    "upgrade", "migrate", "convert", "rewrite", "clean", "finish", "set", "wire", "hook", "connect", "publish",
];
/// Words before the verb in a request: "hey team, please fix…", "can you add…", "I want to build…".
const LEAD_IN: &[&str] = &[
    "hey", "hi", "hello", "team", "please", "pls", "ok", "okay", "so", "now", "can", "could", "would", "will", "you",
    "u", "lets", "let's", "let", "us", "we", "i", "want", "need", "to", "and", "also", "then", "just", "go", "ahead",
];
/// Words that open a question.
const QUESTION_WORDS: &[&str] = &[
    "what", "why", "how", "where", "when", "who", "which", "whats", "what's", "is", "are", "does", "do", "did",
    "was", "were", "explain",
];

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn any_in(words: &[String], list: &[&str]) -> bool {
    words.iter().any(|w| list.contains(&w.as_str()))
}

/// The kind of result `prompt` expects. `recent` is the person's earlier messages in the chat,
/// oldest first: a short follow-up with nothing of its own to go on ("more", "and the other one")
/// expects what the last of them did. A greeting or thank-you is [`Deliverable::Other`].
pub fn expected_kind(prompt: &str, recent: &[&str]) -> Deliverable {
    if super::team::is_small_talk(prompt) {
        return Deliverable::Other;
    }
    classify(prompt).unwrap_or_else(|| {
        let inherited = super::team::reads_as_follow_up(prompt)
            .then(|| recent.iter().rev().find_map(|m| classify(m)))
            .flatten();
        inherited.unwrap_or(Deliverable::Explain)
    })
}

/// The fast plan for every turn, from the message's words alone (no model call): its kind, what
/// to show and the plain checks for it. `who` stays empty; Juno's analyzer fills it for team chats.
pub fn keyword_plan(prompt: &str, recent: &[&str]) -> DeliveryPlan {
    DeliveryPlan::for_kind(expected_kind(prompt, recent), PlanSource::Keywords)
}

/// The kind `text` names on its own, or `None` when nothing in it says.
fn classify(text: &str) -> Option<Deliverable> {
    let words = tokens(text);
    if words.is_empty() {
        return None;
    }
    let joined = words.join(" ");
    let feature_ask = any_in(&words, FEATURE_WORDS) && any_in(&words, FEATURE_ASK);
    if any_in(&words, IDEA_WORDS) || feature_ask || IDEA_PHRASES.iter().any(|p| joined.contains(p)) {
        return Some(Deliverable::Ideas);
    }
    if any_in(&words, SEE_WORDS) || joined.contains("how it looks") || joined.contains("what it looks like") {
        return Some(Deliverable::Visual);
    }
    let first = words.iter().find(|w| !LEAD_IN.contains(&w.as_str()));
    let opens_with_question = words.first().is_some_and(|w| QUESTION_WORDS.contains(&w.as_str()));
    // "Fix differences!", "can you add a footer?": a request, even with a question mark
    if !opens_with_question && first.is_some_and(|w| CHANGE_VERBS.contains(&w.as_str())) {
        return Some(Deliverable::Change);
    }
    if any_in(&words, LOOK_WORDS) {
        return Some(Deliverable::Visual);
    }
    if text.contains('?') || opens_with_question {
        return Some(Deliverable::Explain);
    }
    if any_in(&words, CHANGE_VERBS) {
        return Some(Deliverable::Change);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use Deliverable::*;

    #[test]
    fn his_messages_map_to_what_they_ask_for() {
        // Oleks's messages from the 2026-11-09 chat (release-080/proactive/brief.md, P9)
        let cases = [
            ("Hey team! What some cool feature we can add?", Ideas),
            ("i want to see this ---> mobile sticky booking CTA", Visual),
            ("show me mockup image", Visual),
            ("Fix differences!", Change),
            ("mobile sticky booking CTA -> what is this??", Explain),
        ];
        for (msg, want) in cases {
            let plan = keyword_plan(msg, &[]);
            assert_eq!(plan.kind, want, "{msg}");
            assert_eq!(plan.source, PlanSource::Keywords);
            assert!(plan.who.is_empty());
        }
    }

    #[test]
    fn other_wordings() {
        let cases = [
            ("Any ideas for the landing page?", Ideas),
            ("Suggest a few improvements", Ideas),
            ("what new features should we build next", Ideas),
            ("Screenshot give me!", Visual),
            ("how it looks on a phone", Visual),
            ("can you add a dark mode toggle?", Change),
            ("please update the README", Change),
            ("Fix the design of the footer", Change),
            ("the button is too small, make it bigger", Change),
            ("The design on mobile", Visual),
            ("why does the build fail?", Explain),
            ("How do I fix this?", Explain),
            ("the login page is broken", Explain),
            ("thanks!", Other),
            ("ok, go ahead", Other),
        ];
        for (msg, want) in cases {
            assert_eq!(expected_kind(msg, &[]), want, "{msg}");
        }
    }

    #[test]
    fn a_bare_follow_up_expects_what_the_last_ask_did() {
        let recent = ["hi", "What cool feature could we add?"];
        assert_eq!(expected_kind("more", &recent), Ideas);
        assert_eq!(expected_kind("and the other one", &["show me the header"]), Visual);
        // Its own words win over the earlier ask
        assert_eq!(expected_kind("fix it", &recent), Change);
        // Nothing to go on: a plain answer
        assert_eq!(expected_kind("more", &[]), Explain);
    }

    #[test]
    fn the_hint_renders_from_the_plan() {
        for kind in [Ideas, Visual, Change, Explain] {
            let hint = DeliveryPlan::for_kind(kind, PlanSource::Keywords).hint();
            assert!(hint.starts_with("\n\n[What to deliver] "), "{hint}");
            assert!(!hint.trim().contains('\n'), "one line: {hint}");
        }
        assert_eq!(DeliveryPlan::for_kind(Other, PlanSource::Keywords).hint(), "");
        let ideas = DeliveryPlan::for_kind(Ideas, PlanSource::Keywords);
        assert!(ideas.wants_picture());
        assert!(ideas.hint().contains("Show in the chat: an image, a short text."), "{}", ideas.hint());
        assert!(ideas.hint().contains("Done when: one mockup image per idea is shared in the chat."));
        assert!(DeliveryPlan::for_kind(Visual, PlanSource::Keywords).wants_picture());
        assert!(!DeliveryPlan::for_kind(Change, PlanSource::Keywords).wants_picture());
        // Another planner's plan renders the same way, with its own show and checks
        let analyzer = DeliveryPlan {
            kind: Change,
            show: vec![Show::Link, Show::Image],
            who: vec![("b1".into(), "the API".into())],
            done_when: vec!["the branch is pushed".into()],
            source: PlanSource::Analyzer,
        };
        let hint = analyzer.hint();
        assert!(hint.contains("Show in the chat: a link (the PR or page), an image. Done when: the branch is pushed."), "{hint}");
        // It round-trips as JSON, for the analyzer's output and the turn's record
        let json = serde_json::to_string(&analyzer).unwrap();
        assert!(json.contains("\"kind\":\"change\"") && json.contains("\"source\":\"analyzer\""), "{json}");
        assert_eq!(serde_json::from_str::<DeliveryPlan>(&json).unwrap(), analyzer);
    }
}
