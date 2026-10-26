//! Sticky answers: when a bot in a channel asked the person something and the person replies
//! without an @mention, the reply goes back to that bot instead of being routed afresh. On the live
//! kwant (2026-10-25) Community asked "Which city are you in?", the answer "Munich" was routed to
//! Architect, and Architect set up a routine Community had been asked for.
//!
//! The stickiness ends by itself: only what was said since the person's last message counts, so
//! once the bot has had its answer (and replied without a new question) routing is back to normal.

use crate::db::models::{Kwant, Message};
use crate::db::questions::Question;

/// The bot waiting for the person's answer, and the open `ask_user` question it waits on (if it
/// asked with one).
#[derive(Debug, Clone)]
pub struct Awaiting<'a> {
    pub kwant: &'a Kwant,
    pub question_id: Option<String>,
}

impl PartialEq for Awaiting<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.kwant.id == other.kwant.id && self.question_id == other.question_id
    }
}

/// The bot waiting for the person's answer in a chat, if any. `recent` is the chat oldest first,
/// without the message being routed; `open` are the chat's open `ask_user` questions. Only what
/// came after the person's last message counts:
/// 1. the newest open `ask_user` question asked since then by one of `members`;
/// 2. else the last bot message since then, when it is an `ask_user` card or ends by asking
///    something (and isn't handing on to a teammate with an @mention).
pub fn awaiting_answer<'a>(recent: &[Message], open: &[Question], members: &'a [Kwant]) -> Option<Awaiting<'a>> {
    let member = |id: &str| members.iter().find(|k| k.id == id);
    let last_user = recent.iter().rposition(|m| m.sender_role == "user");
    let since = last_user.and_then(|i| recent[i].created_at);
    let tail = &recent[last_user.map_or(0, |i| i + 1)..];

    let asked = open
        .iter()
        .filter(|q| q.status == "open" && since.is_none_or(|t| q.created_at >= t))
        .filter_map(|q| member(&q.kwant_id).map(|k| (q.created_at, k, q)))
        .max_by_key(|(at, _, _)| *at);
    if let Some((_, kwant, q)) = asked {
        return Some(Awaiting { kwant, question_id: Some(q.id.clone()) });
    }

    let last_bot = tail.iter().rev().find(|m| m.sender_role == "kwant")?;
    let k = member(&last_bot.sender_id)?;
    let card = last_bot
        .meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .is_some_and(|m| m["kind"] == "ask_user" && m["status"] == "open");
    let hands_on = |text: &str| members.iter().any(|m| crate::router::handoff_targets(text, &[m.name.as_str()]).contains(&0));
    (card || (ends_with_question(&last_bot.text) && !hands_on(last_line(&last_bot.text))))
        .then_some(Awaiting { kwant: k, question_id: None })
}

/// The last non-empty line of `text`.
fn last_line(text: &str) -> &str {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// Whether the last line of `text` asks something: a sentence on it ends with "?" (a "?" inside a
/// link's query string doesn't count).
pub fn ends_with_question(text: &str) -> bool {
    let line = last_line(text);
    let chars: Vec<char> = line.chars().collect();
    chars.iter().enumerate().any(|(i, &c)| {
        c == '?' && chars.get(i + 1).is_none_or(|n| n.is_whitespace() || "*_)\"'”’`".contains(*n))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn bot(id: &str) -> Kwant {
        Kwant {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: id.into(),
            name: id.into(),
            title: String::new(),
            persona_prompt: String::new(),
            soul: String::new(),
            avatar_sprite: String::new(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
            temperature: 0.7,
            max_tokens: 4096,
            level: 1,
            turns_completed: 0,
            created_at: None,
        }
    }

    fn msg(at: i64, role: &str, sender: &str, text: &str) -> Message {
        Message {
            id: format!("m{at}"),
            channel_id: Some("c".into()),
            direct_kwant_id: None,
            sender_role: role.into(),
            sender_id: sender.into(),
            text: text.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
            created_at: Some(t(at)),
            thread_id: None,
            meta: None,
        }
    }

    fn t(at: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 25, 22, 0, 0).unwrap() + Duration::seconds(at)
    }

    fn question(at: i64, kwant: &str, status: &str) -> Question {
        Question {
            id: format!("q{at}"),
            kwant_id: kwant.into(),
            channel_id: Some("c".into()),
            direct_kwant_id: None,
            thread_id: None,
            turn_id: None,
            question: "Which city?".into(),
            choices: "[]".into(),
            status: status.into(),
            answer: None,
            message_id: None,
            created_at: t(at),
            answered_at: None,
        }
    }

    fn members() -> Vec<Kwant> {
        vec![bot("Architect"), bot("Community"), bot("Analyst")]
    }

    /// The live case: Community asked for the city, the person's "Munich" belongs to Community.
    #[test]
    fn an_answer_goes_back_to_the_bot_that_asked() {
        let m = members();
        let chat = [
            msg(0, "user", "user", "Every morning send me an overview of my day at 7:00"),
            msg(10, "kwant", "Community", "Happy to set that up. **Which city are you in?**"),
        ];
        assert_eq!(awaiting_answer(&chat, &[], &m).map(|a| a.kwant.id.as_str()), Some("Community"));
    }

    #[test]
    fn an_open_ask_user_question_wins_over_a_later_bot_message() {
        let m = members();
        let chat = [
            msg(0, "user", "user", "plan the launch"),
            msg(5, "kwant", "Community", "Which city are you in?\n\nChoices: Munich · Berlin"),
            msg(9, "kwant", "Analyst", "Here are the numbers."),
        ];
        let open = [question(5, "Community", "open")];
        let a = awaiting_answer(&chat, &open, &m).unwrap();
        assert_eq!((a.kwant.id.as_str(), a.question_id.as_deref()), ("Community", Some("q5")));
        // A question from before the person's last message was already answered in chat
        let open = [question(-5, "Community", "open")];
        assert_eq!(awaiting_answer(&chat, &open, &m), None, "Analyst's last line asks nothing");
        // An answered question doesn't count
        let open = [question(5, "Community", "answered")];
        assert_eq!(awaiting_answer(&chat, &open, &m), None);
    }

    #[test]
    fn an_open_question_card_counts_without_a_question_mark() {
        let m = members();
        let mut card = msg(5, "kwant", "Analyst", "Pick a source\n\nChoices: DWD · Met Office");
        card.meta = Some(r#"{"kind":"ask_user","status":"open"}"#.into());
        let chat = [msg(0, "user", "user", "weather please"), card];
        assert_eq!(awaiting_answer(&chat, &[], &m).map(|a| a.kwant.id.as_str()), Some("Analyst"));
    }

    /// Once the bot that asked has its answer and replies without a new question, routing is normal.
    #[test]
    fn stickiness_ends_once_the_bot_has_answered() {
        let m = members();
        let chat = [
            msg(0, "user", "user", "Every morning send me an overview"),
            msg(10, "kwant", "Community", "Which city are you in?"),
            msg(20, "user", "user", "Munich"),
            msg(30, "kwant", "Community", "Done: every day at 07:00 Europe/Berlin."),
        ];
        assert_eq!(awaiting_answer(&chat, &[], &m), None);
        // Nor does it stick while the person writes twice before the bot replies
        let chat = &chat[..3];
        assert_eq!(awaiting_answer(chat, &[], &m), None);
    }

    #[test]
    fn a_question_to_a_teammate_or_an_old_question_is_not_sticky() {
        let m = members();
        let chat = [msg(0, "user", "user", "go"), msg(5, "kwant", "Community", "@Analyst can you pull last week's numbers?")];
        assert_eq!(awaiting_answer(&chat, &[], &m), None, "a handoff, not a question to the person");
        let chat = [msg(5, "kwant", "Community", "Which city?\n\nI'll wait for that, then set it up.")];
        assert_eq!(awaiting_answer(&chat, &[], &m), None, "the last line asks nothing");
        let chat = [msg(5, "kwant", "Stranger", "Which city?")];
        assert_eq!(awaiting_answer(&chat, &[], &m), None, "not a member of this chat");
        assert_eq!(awaiting_answer(&[], &[], &m), None);
    }

    #[test]
    fn question_marks_in_links_dont_count() {
        assert!(ends_with_question("Should I go ahead?"));
        assert!(ends_with_question("Want the metric version? (I can switch.)"));
        assert!(ends_with_question("**Which city are you in?**"));
        assert!(!ends_with_question("Source: https://example.com/forecast?city=munich"));
        assert!(!ends_with_question("Done."));
        assert!(!ends_with_question(""));
    }
}
