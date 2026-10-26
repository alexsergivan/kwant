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
    /// It asked with an `ask_user` question or card. A plain reply ending in "?" is weaker: a
    /// confident router pick of another bot wins over it ([`yields_to_route`]).
    pub card: bool,
}

impl PartialEq for Awaiting<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.kwant.id == other.kwant.id && self.question_id == other.question_id && self.card == other.card
    }
}

/// The bot waiting for the person's answer in a chat, if any. `recent` is the chat oldest first,
/// without the message being routed; `open` are the chat's open `ask_user` questions. Only what
/// came after the person's last message counts:
/// 0. a list that gathered several bots' questions into one (`meta.consolidated_asks`, see
///    `server::asks`): its bot gets the answer and passes it on;
/// 1. the newest open `ask_user` question asked since then by one of `members`;
/// 2. else the last bot message since then, when it is an `ask_user` card or ends by asking
///    something (and isn't handing on to a teammate with an @mention).
pub fn awaiting_answer<'a>(recent: &[Message], open: &[Question], members: &'a [Kwant]) -> Option<Awaiting<'a>> {
    let member = |id: &str| members.iter().find(|k| k.id == id);
    let last_user = recent.iter().rposition(|m| m.sender_role == "user");
    let since = last_user.and_then(|i| recent[i].created_at);
    let tail = &recent[last_user.map_or(0, |i| i + 1)..];

    let gathered = tail.iter().rev().find_map(|m| {
        let meta = serde_json::from_str::<serde_json::Value>(m.meta.as_deref()?).ok()?;
        let id = meta.get("consolidated_asks")?["kwant_id"].as_str()?.to_string();
        member(&id)
    });
    if let Some(kwant) = gathered {
        return Some(Awaiting { kwant, question_id: None, card: true });
    }

    let asked = open
        .iter()
        .filter(|q| q.status == "open" && since.is_none_or(|t| q.created_at >= t))
        .filter_map(|q| member(&q.kwant_id).map(|k| (q.created_at, k, q)))
        .max_by_key(|(at, _, _)| *at);
    if let Some((_, kwant, q)) = asked {
        return Some(Awaiting { kwant, question_id: Some(q.id.clone()), card: true });
    }

    let last_bot = tail.iter().rev().find(|m| m.sender_role == "kwant")?;
    let k = member(&last_bot.sender_id)?;
    let card = last_bot
        .meta
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .is_some_and(|m| m["kind"] == "ask_user" && m["status"] == "open");
    let hands_on = |text: &str| members.iter().any(|m| crate::router::handoff_targets(text, &[m.name.as_str()]).contains(&0));
    (card || (asks_the_person(&last_bot.text) && !hands_on(last_line(&last_bot.text))))
        .then_some(Awaiting { kwant: k, question_id: None, card })
}

/// Whether a plain-text ask should give way to the router's pick: the router named another bot
/// on a clear match (a role's words, a bot's name, or Laya's confident pick). A greeting, a
/// small reply or no match ("lead", "last", "several") stays with the bot that asked.
pub fn yields_to_route(awaiting: &Awaiting<'_>, route: &crate::router::team::ChannelRoute) -> bool {
    !awaiting.card && route.kwant_id != awaiting.kwant.id && matches!(route.basis.as_str(), "skills" | "keywords")
}

/// Whether the last line of `text` asks the person something real: a question on it that isn't
/// only a polite closer ("Anything else?", "Want me to also add a chart?").
pub fn asks_the_person(text: &str) -> bool {
    questions(last_line(text)).iter().any(|q| !is_closer(q))
}

/// The question the last line of `text` asks the person, if it asks one (see [`asks_the_person`]).
pub fn person_question(text: &str) -> Option<String> {
    questions(last_line(text)).into_iter().rev().find(|q| !is_closer(q)).map(|q| q.trim().to_string())
}

/// Closing offers that end a finished answer rather than wait for one. Matched on the question
/// sentence alone, so "Which city are you in? Anything else?" still asks.
const CLOSERS: &[&str] = &[
    "anything else",
    "anything more",
    "help with anything",
    "help you with anything",
    "want me to also",
    "would you like me to also",
    "shall i also",
    "should i also",
    "should i continue",
    "shall i continue",
    "should i keep going",
    "let me know",
    "help further",
    "help you further",
    "anything further",
    "need more help",
    "need any more help",
    "need further help",
    "any other questions",
    "else i can help",
    "else i can do",
    "else can i help",
    "else can i do",
];

pub fn is_closer(question: &str) -> bool {
    let q: String = question
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' || c == '’' { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    let q = q.split_whitespace().collect::<Vec<_>>().join(" ");
    CLOSERS.iter().any(|c| q.contains(c))
}

/// The question sentences on one line: each runs from the last sentence end to a "?" that ends a
/// sentence (see [`ends_with_question`]).
fn questions(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in chars.iter().enumerate() {
        let ends = chars.get(i + 1).is_none_or(|n| n.is_whitespace() || "*_)\"'”’`".contains(*n));
        if ends && "?.!".contains(c) {
            if c == '?' {
                out.push(chars[start..=i].iter().collect());
            }
            start = i + 1;
        }
    }
    out
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
            why_you: None,
            checked: None,
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

    /// Two bots asked, the lead gathered both into one list: the answer goes to the lead,
    /// not to whichever card is newest.
    #[test]
    fn an_answer_to_a_gathered_list_goes_to_its_lead() {
        let m = members();
        let mut list = msg(30, "kwant", "Architect", "Two things I need from you:\n1. Community: which city?\n2. Analyst: which week?");
        list.meta = Some(r#"{"consolidated_asks":{"kwant_id":"Architect","askers":["Community","Analyst"]}}"#.into());
        let chat = [
            msg(0, "user", "user", "Plan the launch"),
            msg(10, "kwant", "Community", "Which city are you in?"),
            msg(20, "kwant", "Analyst", "Which week should I chart?"),
            list,
        ];
        let open = [question(25, "Analyst", "open")];
        let a = awaiting_answer(&chat, &open, &m).expect("someone waits");
        assert_eq!((a.kwant.id.as_str(), a.question_id, a.card), ("Architect", None, true));
        // Without the list, the newest open card wins as before
        let a = awaiting_answer(&chat[..3], &open, &m).expect("someone waits");
        assert_eq!(a.kwant.id, "Analyst");
    }

    #[test]
    fn the_question_asked_is_the_last_real_one() {
        assert_eq!(person_question("Done.\nWhich city are you in? Anything else?").as_deref(), Some("Which city are you in?"));
        assert_eq!(person_question("Done. Anything else?"), None);
        assert_eq!(person_question("Which week?\nI'll start once you say."), None);
    }

    fn after(reply: &str) -> Option<String> {
        let m = members();
        let chat = [msg(0, "user", "user", "chart last week's signups"), msg(10, "kwant", "Analyst", reply)];
        awaiting_answer(&chat, &[], &m).map(|a| a.kwant.id.clone())
    }

    /// Rigel's #70 review: a finished answer ending in a polite closer trapped the person's next,
    /// unrelated request on the same bot.
    #[test]
    fn closing_questions_dont_stick() {
        for closer in [
            "Here's the chart.\n\nAnything else?",
            "Here's the chart.\n\nWant me to also add a chart?",
            "Done for now. Should I continue?",
            "Here's the chart.\n\nIs there anything else I can help with?",
            "Here's the chart.\n\n**Anything more?**",
            "Here's the chart. Let me know if you need anything else?",
            "Is there anything else I can help you with today?",
            "Would you like me to also send it to the team?",
            "Shall I continue?",
            "Happy to help further?",
            "Here's the chart.\n\nNeed anything else?",
            "Can I help you further with this?",
            "Need any more help with the chart?",
            "Any other questions?",
            "Is there something else I can help with?",
            "What else can I do for you?",
        ] {
            assert_eq!(after(closer), None, "{closer:?}");
        }
    }

    #[test]
    fn a_real_question_still_sticks() {
        for ask in [
            "Happy to.\n\nWhich city are you in?",
            "Which city are you in? Anything else?",
            "Done. Which week should I compare it with?",
            "Should I use DWD or the Met Office?",
        ] {
            assert_eq!(after(ask).as_deref(), Some("Analyst"), "{ask:?}");
        }
        assert!(asks_the_person("Want the metric version? (I can switch.)"));
        assert!(!asks_the_person("Anything else? Let me know."));
    }

    fn route(kwant: &str, basis: &str) -> crate::router::team::ChannelRoute {
        crate::router::team::ChannelRoute {
            kwant_id: kwant.into(),
            reason: String::new(),
            fallback: false,
            matched_terms: vec![],
            basis: basis.into(),
        }
    }

    /// A plain "…?" gives way to a clear match for another bot; a card or open question never does,
    /// and a greeting, a tie or no match stays with the asker.
    #[test]
    fn a_clear_match_for_another_bot_wins_over_a_plain_question() {
        let m = members();
        let chat = [msg(0, "user", "user", "set up my mornings"), msg(10, "kwant", "Community", "Which city are you in?")];
        let a = awaiting_answer(&chat, &[], &m).unwrap();
        assert!(!a.card);
        assert!(yields_to_route(&a, &route("Analyst", "keywords")));
        assert!(yields_to_route(&a, &route("Analyst", "skills")));
        for basis in ["lead", "last", "several", "team", "only"] {
            assert!(!yields_to_route(&a, &route("Analyst", basis)), "{basis}");
        }
        assert!(!yields_to_route(&a, &route("Community", "keywords")), "the router agrees with the asker");

        let open = [question(10, "Community", "open")];
        let a = awaiting_answer(&chat, &open, &m).unwrap();
        assert!(a.card && !yields_to_route(&a, &route("Analyst", "keywords")), "an open question always sticks");
        let mut card = msg(10, "kwant", "Community", "Pick a source");
        card.meta = Some(r#"{"kind":"ask_user","status":"open"}"#.into());
        let chat = [msg(0, "user", "user", "weather"), card];
        let a = awaiting_answer(&chat, &[], &m).unwrap();
        assert!(a.card && !yields_to_route(&a, &route("Analyst", "skills")), "an open card always sticks");
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
