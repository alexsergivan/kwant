//! The next action for a case: a follow-up the person can copy, open in their mail app, or
//! paste on GitHub. Nothing here sends anything; v1 has no SMTP and no GitHub write.

use super::{CaseKind, ConvKind};
use chrono::{DateTime, Utc};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// `email`, `github` or `kwant`: where the person sends it themselves.
    pub channel: String,
    /// The address, the GitHub login, or the bot's name.
    pub to: Option<String>,
    /// Email only: `Re: <original subject>`.
    pub subject: Option<String>,
    pub body: String,
    /// Email only: the Message-ID it answers, for mail apps that thread on it.
    pub in_reply_to: Option<String>,
    /// Email: a `mailto:` link with the subject and body filled in.
    pub mailto: Option<String>,
    /// GitHub: the issue to comment on. kwant: absent (the thread id says where).
    pub open_url: Option<String>,
    /// kwant: the thread to write in.
    pub kwant_thread_id: Option<String>,
    /// `template` or `model`.
    pub generated_by: String,
    pub created_at: DateTime<Utc>,
}

/// What a draft is built from: a stored case's fields.
#[derive(Debug, Clone)]
pub struct DraftInput<'a> {
    pub kind: CaseKind,
    pub conv_kind: ConvKind,
    pub title: &'a str,
    pub counterpart: Option<&'a str>,
    pub counterpart_name: Option<&'a str>,
    pub reference: Option<&'a str>,
    pub link: Option<&'a str>,
    pub source_ref: &'a str,
    pub reply_to_message_id: Option<&'a str>,
    pub last_activity_at: DateTime<Utc>,
    pub deadline: Option<&'a str>,
}

/// Characters a `mailto:` component may keep as they are (RFC 6068 unreserved).
const MAILTO: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// A short greeting name: the display name's first word, else nothing.
fn first_name(name: Option<&str>) -> Option<String> {
    let n = name?.trim().trim_matches('"');
    let first = n.split_whitespace().next()?;
    (first.chars().all(|c| c.is_alphabetic() || c == '-') && !first.contains('@')).then(|| first.to_string())
}

/// " (reference 4471)", or nothing. Said in the body even when the subject has it: whoever
/// reads the reply may not see the subject.
fn reference_clause(reference: Option<&str>) -> String {
    match reference.map(str::trim) {
        Some(r) if !r.is_empty() => format!(" (reference {r})"),
        _ => String::new(),
    }
}

/// The subject with one `Re: ` in front.
pub fn reply_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        // Re (English), AW (German), SV (Nordic): all three letters wide
        if lower.starts_with("re:") || lower.starts_with("aw:") || lower.starts_with("sv:") {
            s = s[3..].trim_start();
        } else {
            break;
        }
    }
    if s.is_empty() {
        "Re: (no subject)".into()
    } else {
        format!("Re: {s}")
    }
}

/// The body of a follow-up for `input`, written as the person.
pub fn template_body(input: &DraftInput) -> String {
    let when = input.last_activity_at.format("%-d %B").to_string();
    let reference = reference_clause(input.reference);
    let about = if input.title.trim().is_empty() || input.title == "(no subject)" {
        "my earlier message".to_string()
    } else {
        format!("\"{}\"", input.title.trim())
    };
    let deadline = input.deadline.map(|d| format!(" You mentioned {d};")).unwrap_or_default();
    match input.conv_kind {
        ConvKind::KwantThread | ConvKind::KwantQuestion | ConvKind::KwantDelegation => {
            return match input.kind {
                CaseKind::FailedTask => format!("Please try this task again: {about}. Tell me what got in the way last time."),
                CaseKind::UnansweredQuestion => "Answering your question now: ".to_string(),
                _ => format!("Picking this back up: {about}. Where did this get to, and what do you need from me to finish it?"),
            };
        }
        ConvKind::Issue | ConvKind::PullRequest => {
            let what = if input.conv_kind == ConvKind::PullRequest { "pull request" } else { "issue" };
            return match input.kind {
                CaseKind::YouOweReply => format!("Sorry for the slow answer here{reference}. "),
                _ => format!(
                    "Following up on this {what}{reference}: is it still relevant? If it is, what is needed to move it forward? \
                     If not, I'll close it."
                ),
            };
        }
        ConvKind::Email => {}
    }
    let hi = match first_name(input.counterpart_name) {
        Some(n) => format!("Hi {n},"),
        None => "Hello,".to_string(),
    };
    let ask = match input.kind {
        CaseKind::PromiseLapsed => format!(
            "I'm following up on {about}{reference}. You said you would get back to me, and I haven't heard anything since {when}.{deadline} \
             Could you let me know where this stands?"
        ),
        CaseKind::YouPromised => format!(
            "Sorry for the delay on {about}{reference}. I said I would follow up and I haven't yet; here it is:\n\n[what you promised]"
        ),
        CaseKind::YouOweReply => format!("Sorry for the slow reply about {about}{reference}.\n\n[your answer]"),
        _ => format!(
            "I'm following up on my message of {when} about {about}{reference}. Could you let me know where this stands?"
        ),
    };
    format!("{hi}\n\n{ask}\n\nThank you,")
}

/// The template draft for `input`.
pub fn template(input: &DraftInput, now: DateTime<Utc>) -> Draft {
    finish(input, template_body(input), "template", now)
}

/// A draft around `body` with the links and addresses `input` gives it.
pub fn finish(input: &DraftInput, body: String, generated_by: &str, now: DateTime<Utc>) -> Draft {
    let mut d = Draft {
        channel: String::new(),
        to: input.counterpart.map(str::to_string),
        subject: None,
        body,
        in_reply_to: None,
        mailto: None,
        open_url: None,
        kwant_thread_id: None,
        generated_by: generated_by.into(),
        created_at: now,
    };
    match input.conv_kind {
        ConvKind::Email => {
            let subject = reply_subject(input.title);
            d.channel = "email".into();
            d.in_reply_to = input.reply_to_message_id.map(str::to_string);
            d.mailto = input.counterpart.filter(|to| to.contains('@') && !to.contains(['?', '&', '\r', '\n'])).map(|to| {
                format!(
                    "mailto:{}?subject={}&body={}",
                    utf8_percent_encode(to, MAILTO),
                    utf8_percent_encode(&subject, MAILTO),
                    utf8_percent_encode(&d.body, MAILTO)
                )
            });
            d.subject = Some(subject);
        }
        ConvKind::Issue | ConvKind::PullRequest => {
            d.channel = "github".into();
            d.open_url = input.link.filter(|l| l.starts_with("https://")).map(str::to_string);
        }
        ConvKind::KwantThread | ConvKind::KwantQuestion | ConvKind::KwantDelegation => {
            d.channel = "kwant".into();
            d.to = input.counterpart_name.or(input.counterpart).map(str::to_string);
            d.kwant_thread_id = input.link.and_then(|l| l.strip_prefix("kwant://thread/")).map(str::to_string);
        }
    }
    d
}

/// Whether a model-written body kept what it must: the reference number, when there is one.
pub fn keeps_reference(body: &str, reference: Option<&str>) -> bool {
    match reference.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => body.contains(r),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(kind: CaseKind, conv_kind: ConvKind) -> DraftInput<'a> {
        DraftInput {
            kind,
            conv_kind,
            title: "Re: Claim 4471 status",
            counterpart: Some("claims@insurer.example"),
            counterpart_name: Some("Dana Smith"),
            reference: Some("4471"),
            link: Some("https://github.com/o/r/issues/7"),
            source_ref: "thread:thread-1",
            reply_to_message_id: Some("<abc@insurer.example>"),
            last_activity_at: "2026-09-09T10:00:00Z".parse().unwrap(),
            deadline: None,
        }
    }

    #[test]
    fn an_email_follow_up_keeps_the_reference_and_threads() {
        let d = template(&input(CaseKind::PromiseLapsed, ConvKind::Email), Utc::now());
        assert_eq!(d.channel, "email");
        assert_eq!(d.subject.as_deref(), Some("Re: Claim 4471 status"));
        assert!(d.body.starts_with("Hi Dana,"), "{}", d.body);
        assert!(d.body.contains("(reference 4471)"), "{}", d.body);
        assert!(d.body.contains("9 September"), "{}", d.body);
        assert_eq!(d.in_reply_to.as_deref(), Some("<abc@insurer.example>"));
        let mailto = d.mailto.unwrap();
        assert!(mailto.starts_with("mailto:claims%40insurer.example?subject=Re%3A%20Claim%204471%20status&body=Hi%20Dana%2C"), "{mailto}");
        assert!(!mailto.contains(' ') && !mailto.contains('\n'));
    }

    #[test]
    fn a_header_injection_address_gets_no_mailto() {
        let mut i = input(CaseKind::AwaitingReply, ConvKind::Email);
        i.counterpart = Some("a@b.example?bcc=evil@x.example");
        assert!(template(&i, Utc::now()).mailto.is_none());
    }

    #[test]
    fn github_and_kwant_drafts_say_where_to_post() {
        let g = template(&input(CaseKind::StaleIssue, ConvKind::Issue), Utc::now());
        assert_eq!(g.channel, "github");
        assert_eq!(g.open_url.as_deref(), Some("https://github.com/o/r/issues/7"));
        assert!(g.subject.is_none() && g.mailto.is_none());
        let mut ki = input(CaseKind::FailedTask, ConvKind::KwantDelegation);
        ki.link = Some("kwant://thread/thread-1");
        let k = template(&ki, Utc::now());
        assert_eq!(k.channel, "kwant");
        assert_eq!(k.kwant_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(k.to.as_deref(), Some("Dana Smith"));
    }

    #[test]
    fn reply_subjects() {
        assert_eq!(reply_subject("Re: RE: hello"), "Re: hello");
        assert_eq!(reply_subject("AW: Angebot"), "Re: Angebot");
        assert_eq!(reply_subject(""), "Re: (no subject)");
    }

    #[test]
    fn a_model_draft_must_keep_the_reference() {
        assert!(keeps_reference("about claim 4471", Some("4471")));
        assert!(!keeps_reference("about your claim", Some("4471")));
        assert!(keeps_reference("anything", None));
    }
}
