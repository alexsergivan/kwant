//! Dev mode work from Jira (0.7.6, next-076/jira-tickets): Vega's half. Juno's pickup calls
//! [`hooks::start`] when a ticket's turn comes; the team's lead reads it in a turn of its own
//! ([`tools`], the brief turn), hands in a cited brief ([`brief`]), kwant asks the person what blocks
//! (the questions in [`hooks`]) and then starts the work ([`mission`]). The review of the PR checks
//! it against the brief's done-when lines (`dev::review`, Matches the ticket).
//!
//! With no Jira site connected nothing here runs: no hook is called, no tool is offered, and the
//! review finds no brief for any PR.

pub mod brief;
pub mod brief_routes;
pub mod hooks;
pub mod links;
pub mod mission;
pub mod prompt;
pub mod seam;
pub mod source;
pub mod tools;

/// The brief turn's purpose (`TurnOpts::purpose`): it runs only on Claude with kwant's permission
/// hook, gets only the four brief tools and only reads ([`tools::BriefApprover`]).
pub const BRIEF_PURPOSE: &str = "dev_brief";

/// Whether a brief turn may run on this engine (J7): Claude, whose permission hook holds every
/// call to reading, and Codex, in its read-only sandbox with the gateway's calls held the same
/// way. Grok and Custom API accounts can be neither gated nor sandboxed, so they never read a
/// ticket.
pub fn brief_engine(engine: crate::profiles::Engine) -> bool {
    matches!(engine, crate::profiles::Engine::Claude | crate::profiles::Engine::Codex)
}

/// A Jira key as kwant accepts it: `PROJECT-123`, upper-case, at most 40 characters.
pub fn valid_key(key: &str) -> bool {
    let Some((project, n)) = key.split_once('-') else { return false };
    key.len() <= 40
        && project.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && project.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !n.is_empty()
        && n.len() <= 9
        && n.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys() {
        assert!(super::valid_key("TRAIL-102"));
        assert!(super::valid_key("AB_2-1"));
        assert!(!super::valid_key("trail-102"));
        assert!(!super::valid_key("TRAIL-"));
        assert!(!super::valid_key("TRAIL-1-2"));
        assert!(!super::valid_key("../x-1"));
        assert!(!super::valid_key("TRAIL102"));
    }
}

#[cfg(test)]
mod engine_tests {
    use super::brief_engine;
    use crate::profiles::Engine;

    #[test]
    fn a_brief_reads_a_ticket_only_on_an_engine_that_can_be_held_to_reading() {
        assert!(brief_engine(Engine::Claude));
        assert!(brief_engine(Engine::Codex), "in its read-only sandbox");
        assert!(!brief_engine(Engine::Grok));
        assert!(!brief_engine(Engine::Custom));
    }
}
