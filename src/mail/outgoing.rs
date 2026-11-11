//! The mail kwant sends, built and checked in the core before any provider sees it: the send
//! list, the loop marks, no control characters in a header, plain text only. And the words of
//! each mail (Mira's spec §6: M0 test, M1/M2 question, M3 confirmation, M4 too late, M5 unclear).

use super::safety::{
    base_address, check_recipients, clean_line, clean_text, has_control_chars, parse_addresses,
    LOOP_MARK,
};
use super::OutgoingMail;
use lettre::message::header::{ContentType, Header, HeaderName, HeaderValue};
use lettre::message::Mailbox;
use lettre::Message;

/// `Auto-Submitted: auto-generated` (RFC 3834): auto-responders don't answer it.
#[derive(Clone)]
struct AutoSubmitted;

impl Header for AutoSubmitted {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("Auto-Submitted")
    }
    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(AutoSubmitted)
    }
    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), "auto-generated".into())
    }
}

/// `X-Loop: kwant`: another kwant (or this one) never treats it as a person's mail.
#[derive(Clone)]
struct XLoop;

impl Header for XLoop {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("X-Loop")
    }
    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(XLoop)
    }
    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), LOOP_MARK.into())
    }
}

/// A mail to build: every field is checked by [`build`].
#[derive(Debug, Clone, Default)]
pub struct Draft {
    /// Shown name, e.g. `Drafter (kwant)`.
    pub from_name: String,
    pub from: String,
    pub to: Vec<String>,
    /// Where replies go; kwant sets it to `from`, never to an address a mail gave it.
    pub reply_to: Option<String>,
    pub subject: String,
    pub text: String,
    /// Without angle brackets, e.g. `q_ab12.x7k@kwant`.
    pub message_id: String,
    pub in_reply_to: Option<String>,
}

pub const MAX_SUBJECT: usize = 200;
pub const MAX_TEXT: usize = 20_000;

fn one_address(a: &str) -> Result<lettre::Address, String> {
    if has_control_chars(a) || parse_addresses(a).len() != 1 {
        return Err(format!("not one address: {}", clean_line(a)));
    }
    a.trim()
        .parse::<lettre::Address>()
        .map_err(|_| format!("not an address: {}", clean_line(a)))
}

fn message_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id.contains('@')
        && id
            .chars()
            .all(|c| c.is_ascii_graphic() && !"<>\"\\ ".contains(c))
}

/// The message as bytes, or why not: the recipients must be on the send list (or our own
/// addresses), the header values free of control characters.
pub fn build(d: &Draft, send_list: &[String], own: &[String]) -> Result<OutgoingMail, String> {
    let v = check_recipients(&d.to, send_list, own);
    if !v.ok {
        return Err(v.reason.unwrap_or_else(|| "recipient refused".into()));
    }
    for h in [&d.from_name, &d.subject, &d.message_id]
        .into_iter()
        .chain(d.in_reply_to.iter())
        .chain(d.reply_to.iter())
    {
        if has_control_chars(h) || h.contains(['\r', '\n']) {
            return Err("a header has control characters".into());
        }
    }
    if !message_id_ok(&d.message_id) || d.in_reply_to.as_deref().is_some_and(|i| !message_id_ok(i))
    {
        return Err("bad message id".into());
    }
    let from = one_address(&d.from)?;
    let name = clean_line(&d.from_name)
        .chars()
        .take(80)
        .collect::<String>();
    let subject: String = clean_line(&d.subject).chars().take(MAX_SUBJECT).collect();
    let text: String = clean_text(&d.text).chars().take(MAX_TEXT).collect();
    let mut b = Message::builder()
        .from(Mailbox::new(
            Some(name).filter(|n| !n.is_empty()),
            from.clone(),
        ))
        .subject(subject)
        .message_id(Some(format!("<{}>", d.message_id)))
        .header(AutoSubmitted)
        .header(XLoop)
        .header(ContentType::TEXT_PLAIN);
    let mut to = Vec::new();
    for t in &d.to {
        let a = one_address(t)?;
        to.push(a.clone());
        b = b.to(Mailbox::new(None, a));
    }
    if let Some(r) = &d.reply_to {
        b = b.reply_to(Mailbox::new(None, one_address(r)?));
    }
    if let Some(i) = &d.in_reply_to {
        b = b.in_reply_to(format!("<{i}>")).references(format!("<{i}>"));
    }
    let msg = b
        .body(text)
        .map_err(|e| format!("could not build the mail: {e}"))?;
    Ok(OutgoingMail {
        envelope_from: from.to_string(),
        envelope_to: to.iter().map(|a| a.to_string()).collect(),
        bytes: msg.formatted(),
    })
}

/// The question's first line, cut at a word to 70 characters (with "…").
pub fn ask_subject(question: &str) -> String {
    let first = clean_line(
        question
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("A question"),
    );
    if first.chars().count() <= 70 {
        return first;
    }
    let cut: String = first.chars().take(70).collect();
    let at = cut.rfind(' ').filter(|&i| i >= 20).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end())
}

fn link_lines(link: Option<&str>, label: &str) -> String {
    match link {
        Some(l) => format!("\n\n{label}\n{l}"),
        None => String::new(),
    }
}

fn numbered(choices: &[String]) -> String {
    choices
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{}. {}", i + 1, clean_line(c)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// M1 (with options) and M2 (without): the question by email.
pub fn ask_text(
    bot: &str,
    place: &str,
    question: &str,
    choices: &[String],
    free_text: bool,
    send_list: &[String],
    link: Option<&str>,
) -> String {
    let rest: String = {
        let mut lines = question.lines().skip_while(|l| l.trim().is_empty());
        lines.next();
        clean_text(&lines.collect::<Vec<_>>().join("\n"))
            .trim()
            .chars()
            .take(600)
            .collect()
    };
    let mut out = format!(
        "{} in {} is waiting on this.\n\n{}",
        clean_line(bot),
        clean_line(place),
        clean_line(
            question
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
        )
    );
    if !rest.is_empty() {
        out.push_str(&format!("\n\n{rest}"));
    }
    if choices.is_empty() {
        out.push_str("\n\nReply to this email with your answer.");
    } else {
        let head = if free_text {
            "Reply with a number or your own answer:"
        } else {
            "Reply with a number:"
        };
        out.push_str(&format!("\n\n{head}\n\n{}", numbered(choices)));
    }
    out.push_str(&link_lines(link, "Or answer in kwant:"));
    let who = send_list
        .iter()
        .map(|a| base_address(a))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "\n\n—\nOnly replies from {who} count. kwant reads the top of your reply and ignores the quoted email. \
         A reply can answer this question; it can't approve anything or start new work.\n\
         Stop these emails: kwant › Settings › Connections › Bot email."
    ));
    out
}

/// M0: the setup test.
pub fn test_text(mailbox: &str) -> String {
    format!(
        "This is kwant checking it can send from {}.\n\nTo finish setting up email asks, reply to this email with: ok\n\n\
         You can ignore this if you only want sign-in codes.\n\n—\nSent because someone set up Bot email in kwant with this address. \
         If that wasn't you, ignore this email.",
        clean_line(mailbox)
    )
}

pub const TEST_SUBJECT: &str = "kwant test: reply ok";

/// M3: the answer was used.
pub fn confirm_text(bot: &str, answer: &str, link: Option<&str>) -> String {
    let a: String = clean_line(answer).chars().take(200).collect();
    format!(
        "Got it: {a}. {} is carrying on.{}\n\n—\nYou can't reply to this one.",
        clean_line(bot),
        link_lines(link, "See it in kwant:")
    )
}

/// M4: the question was already answered.
pub fn too_late_text(answer: &str, at: &str, link: Option<&str>) -> String {
    let a: String = clean_line(answer).chars().take(200).collect();
    format!("This was already answered in kwant ({a}, at {at}), so your reply wasn't used. Nothing changed.{}\n\n—\nYou can't reply to this one.", link_lines(link, "See it in kwant:"))
}

/// M5: the reply picked no single option.
pub fn unclear_text(reply: &str, choices: &[String], link: Option<&str>) -> String {
    let r: String = clean_line(reply).chars().take(80).collect();
    format!("Your reply \"{r}\" didn't pick one answer, so it wasn't used.\n\nReply with one number:\n\n{}{}", numbered(choices), link_lines(link, "Or answer in kwant:"))
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::*;

    fn draft() -> Draft {
        Draft {
            from_name: "Drafter (kwant)".into(),
            from: "bots+kwant-drafter-k7q2m@inbox.example.dev".into(),
            to: vec!["owner@example.org".into()],
            reply_to: Some("bots+kwant-drafter-k7q2m@inbox.example.dev".into()),
            subject: "Which day should the launch post go out?".into(),
            text: "Reply with a number".into(),
            message_id: "q_ab12.x7k2m@kwant".into(),
            in_reply_to: None,
        }
    }

    fn own() -> Vec<String> {
        vec!["bots@inbox.example.dev".into()]
    }

    #[test]
    fn every_mail_carries_the_loop_marks_and_reads_back() {
        let m = build(&draft(), &["owner@example.org".into()], &own()).unwrap();
        assert_eq!(m.envelope_to, vec!["owner@example.org".to_string()]);
        let p = parse(&m.bytes, &[]).unwrap();
        assert_eq!(p.mail.header("auto-submitted"), "auto-generated");
        assert_eq!(p.mail.header("x-loop"), "kwant");
        assert_eq!(p.message_id.as_deref(), Some("q_ab12.x7k2m@kwant"));
        assert_eq!(p.mail.subject, "Which day should the launch post go out?");
        // Read back by kwant, it is kwant's own automated mail, never an answer
        assert_eq!(
            super::super::safety::automation(&p.mail).as_deref(),
            Some(super::super::safety::AGENT_MAIL)
        );
    }

    #[test]
    fn only_to_the_send_list_or_our_own_addresses() {
        let send = vec!["owner@example.org".to_string()];
        let mut d = draft();
        d.to = vec!["someone@else.example".into()];
        assert!(build(&d, &send, &own()).is_err());
        d.to = vec![];
        assert!(build(&d, &send, &own()).is_err());
        // An own +tag address (the setup test reads its own mail back)
        d.to = vec!["bots+kwant-test-x3d9a@inbox.example.dev".into()];
        assert!(build(&d, &send, &own()).is_ok());
        // An empty send list sends nowhere but to ourselves
        d.to = vec!["owner@example.org".into()];
        assert!(build(&d, &[], &own()).is_err());
    }

    #[test]
    fn no_control_characters_or_header_injection() {
        let send = vec!["owner@example.org".to_string()];
        for bad in ["Hi\r\nBcc: x@evil.example", "Hi\u{0}", "Hi\u{1b}[31m"] {
            let mut d = draft();
            d.subject = bad.into();
            assert!(build(&d, &send, &own()).is_err(), "{bad:?}");
            let mut d = draft();
            d.from_name = bad.into();
            assert!(build(&d, &send, &own()).is_err(), "{bad:?}");
        }
        let mut d = draft();
        d.to = vec!["owner@example.org, x@evil.example".into()];
        assert!(build(&d, &send, &own()).is_err());
        let mut d = draft();
        d.message_id = "a b@c".into();
        assert!(build(&d, &send, &own()).is_err());
    }

    #[test]
    fn a_reply_is_threaded() {
        let mut d = draft();
        d.in_reply_to = Some("CAB12x@mail.example.org".into());
        let m = build(&d, &["owner@example.org".into()], &own()).unwrap();
        let p = parse(&m.bytes, &[]).unwrap();
        // Message-IDs are compared without case on both sides (parse and inbox::answer_for)
        assert!(
            p.refers_to.iter().any(|r| r == "cab12x@mail.example.org"),
            "{:?}",
            p.refers_to
        );
    }

    #[test]
    fn subjects_and_texts() {
        assert_eq!(ask_subject("Ship it?\nMore context"), "Ship it?");
        let long = "Which of these three launch dates works best for the post about the spring catalogue and the sale?";
        let s = ask_subject(long);
        assert!(s.ends_with('…') && s.chars().count() <= 71, "{s}");
        assert!(!s.contains("  "));
        let t = ask_text(
            "Drafter",
            "My space › #launch",
            "Which day?\nThe post is ready.",
            &["Monday".into(), "Wednesday".into()],
            true,
            &["owner@example.org".into()],
            None,
        );
        assert!(t.contains("1. Monday\n2. Wednesday"), "{t}");
        assert!(t.contains("The post is ready."));
        assert!(
            !t.contains("Or answer in kwant"),
            "no link line without a base URL"
        );
        let t = ask_text(
            "Drafter",
            "My space",
            "Anything to add?",
            &[],
            true,
            &["owner@example.org".into()],
            Some("https://kwant.example/q/1"),
        );
        assert!(
            t.contains("Reply to this email with your answer.")
                && t.contains("Or answer in kwant:\nhttps://kwant.example/q/1")
        );
        assert!(unclear_text("2 or 3", &["a".into(), "b".into()], None).contains("\"2 or 3\""));
        assert!(confirm_text("Drafter", "Wednesday", None)
            .starts_with("Got it: Wednesday. Drafter is carrying on."));
        assert!(too_late_text("Monday", "10:22", None).contains("(Monday, at 10:22)"));
        assert!(test_text("bots@inbox.example.dev").contains("reply to this email with: ok"));
    }
}
