//! A raw mail (RFC 5322 bytes from IMAP) to the [`Mail`] the safety rules read. Header values
//! are kept as written (unfolded), so the From check sees the same header any other reader
//! would; the DMARC result is read only from a server we trust.

use super::auth_results::trusted_dmarc;
use super::html::html_to_text;
use super::safety::{clean_line, parse_addresses, Mail};
use mail_parser::{MessageParser, PartType};

/// Most header fields read from one mail; a mail with more is read up to here.
const MAX_HEADERS: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub mail: Mail,
    /// Every address it was sent to: To, Cc, Delivered-To and X-Original-To, lowercase.
    pub recipients: Vec<String>,
    pub message_id: Option<String>,
    /// The Message-IDs it answers (In-Reply-To and References), without angle brackets.
    pub refers_to: Vec<String>,
}

fn unfold(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut lines = v.split('\n');
    if let Some(first) = lines.next() {
        out.push_str(first.trim_end_matches('\r'));
    }
    for l in lines {
        let l = l.trim_end_matches('\r');
        if l.starts_with([' ', '\t']) {
            out.push(' ');
            out.push_str(l.trim_start());
        } else if !l.is_empty() {
            out.push(' ');
            out.push_str(l);
        }
    }
    out.trim().to_string()
}

fn ids(v: &str) -> Vec<String> {
    v.split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| {
            t.trim_matches(|c| c == '<' || c == '>')
                .to_ascii_lowercase()
        })
        .filter(|t| !t.is_empty() && t.contains('@') && t.len() <= 250)
        .collect()
}

/// Reads one mail. `trusted` names the authserv-ids whose Authentication-Results count.
pub fn parse(bytes: &[u8], trusted: &[String]) -> Option<Parsed> {
    let m = MessageParser::default().parse(bytes)?;
    let raw = m.raw_message.as_ref();
    let mut headers: Vec<(String, String)> = Vec::new();
    for h in m.parts.first()?.headers.iter().take(MAX_HEADERS) {
        let (start, end) = (h.offset_start as usize, h.offset_end as usize);
        let value = raw
            .get(start..end)
            .map(|b| unfold(&String::from_utf8_lossy(b)))
            .unwrap_or_default();
        headers.push((h.name.as_str().to_ascii_lowercase(), value));
    }
    let mut text = String::new();
    let mut html: Option<String> = None;
    for id in &m.text_body {
        if let Some(PartType::Text(t)) = m.parts.get(*id as usize).map(|p| &p.body) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    for id in &m.html_body {
        if let Some(PartType::Html(h)) = m.parts.get(*id as usize).map(|p| &p.body) {
            html.get_or_insert_with(String::new).push_str(h);
        }
    }
    if text.trim().is_empty() {
        if let Some(h) = &html {
            text = html_to_text(h);
        }
    }
    let first = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let from = first("from").and_then(|f| {
        let a = parse_addresses(&f);
        (a.len() == 1).then(|| a[0].clone())
    });
    let mail_from = first("return-path").map(|r| {
        r.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim()
            .to_string()
    });
    let mut recipients: Vec<String> = Vec::new();
    for (n, v) in &headers {
        if matches!(n.as_str(), "to" | "cc" | "delivered-to" | "x-original-to") {
            for a in parse_addresses(v) {
                if !recipients.contains(&a) {
                    recipients.push(a);
                }
            }
        }
    }
    let mut refers_to: Vec<String> = Vec::new();
    for (n, v) in &headers {
        if n == "in-reply-to" || n == "references" {
            refers_to.extend(ids(v));
        }
    }
    let message_id = first("message-id").and_then(|v| ids(&v).into_iter().next());
    let mut mail = Mail {
        headers,
        from,
        mail_from,
        dmarc: None,
        subject: clean_line(m.subject().unwrap_or("")),
        text,
        html,
    };
    mail.dmarc = trusted_dmarc(&mail, trusted);
    Some(Parsed {
        mail,
        recipients,
        message_id,
        refers_to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOGLE: &str = "mx.google.com";

    fn raw(extra: &str, body: &str) -> Vec<u8> {
        format!(
            "Return-Path: <owner@example.org>\r\nDelivered-To: bots+kwant-drafter-k7q2m@example.com\r\n\
             Authentication-Results: mx.google.com;\r\n       dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=example.org\r\n\
             From: Owner <owner@example.org>\r\nTo: Drafter <bots+kwant-drafter-k7q2m@example.com>\r\n\
             Subject: =?UTF-8?Q?Re:_Which_day=3F?=\r\nMessage-ID: <abc@mail.example.org>\r\n\
             In-Reply-To: <q_1.x@kwant>\r\n{extra}\r\n{body}"
        )
        .into_bytes()
    }

    #[test]
    fn reads_headers_recipients_and_a_trusted_dmarc_result() {
        let p = parse(
            &raw("Content-Type: text/plain; charset=utf-8\r\n", "2\r\n"),
            &[GOOGLE.into()],
        )
        .unwrap();
        assert_eq!(p.mail.from.as_deref(), Some("owner@example.org"));
        assert_eq!(p.mail.mail_from.as_deref(), Some("owner@example.org"));
        assert_eq!(p.mail.subject, "Re: Which day?");
        assert_eq!(p.mail.text.trim(), "2");
        assert_eq!(p.mail.header("from"), "Owner <owner@example.org>");
        assert_eq!(
            p.recipients,
            vec!["bots+kwant-drafter-k7q2m@example.com".to_string()]
        );
        assert_eq!(p.message_id.as_deref(), Some("abc@mail.example.org"));
        assert_eq!(p.refers_to, vec!["q_1.x@kwant".to_string()]);
        let d = p.mail.dmarc.unwrap();
        assert_eq!((d.result.as_str(), d.aligned), ("pass", true));
        // No trusted server: no result at all
        assert_eq!(
            parse(&raw("", "2"), &["mx.example.net".into()])
                .unwrap()
                .mail
                .dmarc,
            None
        );
    }

    #[test]
    fn an_html_only_mail_gets_text_and_keeps_its_html() {
        let p = parse(
            &raw(
                "Content-Type: text/html; charset=utf-8\r\n",
                "<p>Your code is <b>482913</b></p>",
            ),
            &[],
        )
        .unwrap();
        assert_eq!(p.mail.text, "Your code is 482913");
        assert!(p.mail.html.unwrap().contains("<b>482913</b>"));
    }

    #[test]
    fn a_subject_with_line_breaks_or_escapes_is_one_plain_line() {
        let p = parse(
            b"From: a@b.com\r\nSubject: =?UTF-8?Q?Hi=0D=0ABcc:_x=1B[31m?=\r\n\r\nx",
            &[],
        )
        .unwrap();
        assert!(
            !p.mail.subject.chars().any(char::is_control),
            "{:?}",
            p.mail.subject
        );
    }

    #[test]
    fn two_from_addresses_leave_the_field_empty_and_the_header_as_written() {
        let p = parse(b"From: a@b.com, c@d.com\r\n\r\nx", &[]).unwrap();
        assert_eq!(p.mail.from, None);
        assert_eq!(p.mail.header("from"), "a@b.com, c@d.com");
    }
}
