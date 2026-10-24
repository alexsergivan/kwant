//! Prompts and reply conventions for moderated team discussions: the lead opens (or asks the user
//! first), teammates react to each other briefly or pass, and the lead closes with one summary.

/// A teammate with nothing new to add replies with this instead of posting.
pub const PASS: &str = "[PASS]";
/// The opening speaker starts its reply with this to pause the discussion for the user's input.
pub const ASK_USER: &str = "[ASK_USER]";

/// Prefix of `reasoning_content` on intermediate discussion turns; the UI folds these into a thread.
pub const TURN_LABEL: &str = "Team Discussion";
/// Prefix of `reasoning_content` on the closing summary.
pub const SUMMARY_LABEL: &str = "Team Summary";

/// Every discussion turn reads this: look things up before asking the user or leaving a question open.
pub const RESEARCH_FIRST: &str = "Research before you ask: when something is unclear (the audience, the price, what a product does), \
first try to answer it with your tools: open the site or URL in the user's message and its pricing, docs and about pages, \
search the web, read the files, your notes and earlier chats. Use what you find and name the source in a few words.";

/// Turns allowed per discussion, not counting the closing summary.
pub const MAX_TURNS: usize = 5;
/// How often one kwant may speak in a discussion (a reply when a teammate @mentions them back).
pub const MAX_TURNS_PER_KWANT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Speak(String),
    Pass,
    AskUser(String),
}

pub fn parse_reply(text: &str) -> Reply {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.to_ascii_uppercase().starts_with(PASS) {
        return Reply::Pass;
    }
    if trimmed.contains(ASK_USER) {
        let question = trimmed.replace(ASK_USER, "").trim().to_string();
        return if question.is_empty() { Reply::Pass } else { Reply::AskUser(question) };
    }
    Reply::Speak(trimmed.replace(PASS, "").trim().to_string())
}

pub fn turn_label(turn: usize, team: &str) -> String {
    format!("{} • Turn {} • @{}", TURN_LABEL, turn, team)
}

pub fn summary_label(team: &str, contributors: &[String]) -> String {
    format!("{} • @{} • with {}", SUMMARY_LABEL, team, contributors.join(", "))
}

pub fn question_label(speaker: &str, team: &str) -> String {
    format!("{} needs your input before the team continues • @{}", speaker, team)
}

pub fn opening_prompt(user_prompt: &str, team: &str, responders: &[String]) -> String {
    let next = if responders.is_empty() {
        "You'll then wrap up with a short summary.".to_string()
    } else {
        format!(
            "{} will react to your points next (they get the turn automatically, so don't @mention them), then you'll wrap up with a short summary.",
            responders.join(" and ")
        )
    };
    format!(
        "You're opening a team discussion in @{team} about the user's latest message. {next}\n\n\
        User's message:\n{user_prompt}\n\n\
        How to open:\n\
        - {research}\n\
        - Only if you looked and still can't make progress without something only the user knows or decides (budget, priorities, approvals), reply with {ask} followed by at most 2 short questions and one line on what you checked; the team then waits for the user's answer. Prefer stating a reasonable assumption in one line and moving on.\n\
        - Otherwise give your take from your role: concrete and numbered, so teammates can react to specific points. Start with what you found if you looked something up.\n\
        - Under 250 words. No preamble, don't introduce yourself, don't write the final answer yet.",
        research = RESEARCH_FIRST,
        ask = ASK_USER,
    )
}

pub fn reply_prompt(user_prompt: &str, team: &str, previous_speaker: &str, asked_by: Option<&str>) -> String {
    let cue = match asked_by {
        Some(from) => format!("{from} asked you directly; answer them first."),
        None => format!("{previous_speaker} just spoke."),
    };
    format!(
        "Team discussion in @{team} about the user's message (see the conversation above). {cue}\n\n\
        User's message:\n{user_prompt}\n\n\
        Your turn:\n\
        - React to what teammates said, naming them without @: agree, challenge, or refine specific points. Don't restart from scratch or repeat them.\n\
        - Add only what your expertise brings that's still missing.\n\
        - {research} If a teammate left a question that a lookup answers, answer it.\n\
        - Under 200 words. No preamble.\n\
        - Only if one teammate's input is truly needed, @mention that one teammate with a specific question.\n\
        - If you have nothing meaningful to add, reply with exactly {pass} and nothing else.",
        research = RESEARCH_FIRST,
        pass = PASS,
    )
}

pub fn summary_prompt(user_prompt: &str, contributors: &[String]) -> String {
    format!(
        "Wrap up the team discussion above (contributors: {}). Write the team's single final answer to the user's message:\n{user_prompt}\n\n\
        - Lead with the answer or recommendation; merge the best points and resolve disagreements, crediting teammates by name where it helps.\n\
        - Bullets, under 300 words.\n\
        - {research}\n\
        - Never leave open a question that the user's message, a URL in it, the discussion above or a quick lookup can answer: \
        look it up and give the answer with its source, or state the assumption you made.\n\
        - End with \"Open questions:\" only for real decisions the user must make (budget, priorities, approvals), and say what you checked.\n\
        - No @mentions and no commentary about the discussion itself.",
        contributors.join(", "),
        research = RESEARCH_FIRST,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pass_ask_and_speak() {
        assert_eq!(parse_reply("  [PASS]  "), Reply::Pass);
        assert_eq!(parse_reply("[pass] nothing to add"), Reply::Pass);
        assert_eq!(parse_reply(""), Reply::Pass);
        assert_eq!(
            parse_reply("[ASK_USER] Who is the audience?"),
            Reply::AskUser("Who is the audience?".into())
        );
        assert_eq!(parse_reply("I agree with TechLead."), Reply::Speak("I agree with TechLead.".into()));
    }

    #[test]
    fn every_discussion_prompt_says_research_before_asking() {
        let team = vec!["Writer".to_string()];
        for prompt in [
            opening_prompt("Plan kwant.si marketing", "core", &team),
            reply_prompt("Plan kwant.si marketing", "core", "Strategist", None),
            summary_prompt("Plan kwant.si marketing", &team),
        ] {
            assert!(prompt.contains(RESEARCH_FIRST), "{prompt}");
        }
        let open = opening_prompt("Plan kwant.si marketing", "core", &team);
        assert!(open.contains("Only if you looked and still can't"), "{open}");
    }

    #[test]
    fn summary_keeps_open_questions_for_real_decisions_only() {
        let summary = summary_prompt("Create a marketing plan for kwant.si", &["Strategist".into(), "Writer".into()]);
        assert!(summary.contains("Never leave open a question that the user's message, a URL in it"), "{summary}");
        assert!(summary.contains("only for real decisions the user must make"), "{summary}");
        assert!(!summary.contains("only if decisions remain for the user"), "{summary}");
    }

    #[test]
    fn labels_share_prefixes_the_ui_groups_on() {
        assert!(turn_label(2, "core").starts_with(TURN_LABEL));
        assert!(summary_label("core", &["A".into(), "B".into()]).starts_with(SUMMARY_LABEL));
        assert!(!question_label("A", "core").starts_with(TURN_LABEL));
    }
}
