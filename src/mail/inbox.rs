//! Reading a bot mailbox through any [`MailProvider`]: fetch raw mail, parse it with the
//! provider's trusted authserv-ids, and decide each mail with [`super::flow`]. This is the one
//! path every provider's mail takes, so a hosted provider (phase 3) gets the same checks as
//! IMAP: the provider never decides what is safe.

use super::flow::{answer_in, code_in};
use super::parse::{parse, Parsed};
use super::safety::{base_address, single_from, Found};
use super::{FetchQuery, MailError, MailProvider};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;

/// Mail that arrived this long before a wait started still counts (server clocks differ).
pub const CLOCK_SLACK_SECS: i64 = 120;

/// One fetched mail, parsed (or not: unreadable mail is never anyone's).
pub struct Polled {
    pub key: String,
    pub received: Option<DateTime<Utc>>,
    pub parsed: Option<Parsed>,
}

/// New mail since `since` (minus the slack), sent to one of `addresses` when given, minus the
/// keys in `skip`.
pub async fn poll(
    p: &dyn MailProvider,
    addresses: Option<Vec<String>>,
    since: DateTime<Utc>,
    skip: &HashSet<String>,
) -> Result<Vec<Polled>, MailError> {
    let from = since - Duration::seconds(CLOCK_SLACK_SECS);
    let q = FetchQuery {
        since: Some(from),
        addresses,
        skip: skip.clone(),
    };
    let trusted = p.trusted_authserv();
    let mut out = Vec::new();
    for raw in p.fetch(&q).await? {
        if skip.contains(&raw.key) || raw.received.is_some_and(|r| r < from) {
            continue;
        }
        let parsed = parse(&raw.bytes, &trusted);
        out.push(Polled {
            key: raw.key,
            received: raw.received,
            parsed,
        });
    }
    Ok(out)
}

/// A site as a bot names it (`https://www.tiktok.com/signup`) to the domain codes come from
/// (`tiktok.com`), or `None` when it isn't a host name.
pub fn site_domain(site: &str) -> Option<String> {
    let s = site.trim().to_ascii_lowercase();
    let s = s.split("://").nth(1).unwrap_or(&s).to_string();
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("").trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    let ok = host.contains('.')
        && host.len() <= 253
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ok.then(|| host.to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodeOutcome {
    /// Only the code or link, never the mail.
    Code(Found),
    /// For this wait, but refused (the reason is kwant's, never the mail's text).
    Refused(String),
    /// Not for this wait: left for another.
    NotOurs,
}

/// Whether a mail holds the code a bot waits for at `address`. With one shared address
/// (`tagged` false), only mail from `site` is this wait's.
pub fn code_for(m: &Polled, address: &str, tagged: bool, site: Option<&str>) -> CodeOutcome {
    let Some(p) = &m.parsed else {
        return if tagged {
            CodeOutcome::Refused("unreadable mail".into())
        } else {
            CodeOutcome::NotOurs
        };
    };
    if tagged {
        let a = address.to_ascii_lowercase();
        if !p.recipients.contains(&a) {
            return CodeOutcome::NotOurs;
        }
    }
    match code_in(&p.mail, site) {
        Ok(found) => CodeOutcome::Code(found),
        Err(r) if !tagged && r.starts_with("not from ") => CodeOutcome::NotOurs,
        Err(r) => CodeOutcome::Refused(r),
    }
}

/// A question waiting for its answer by email: the address it went out from and its Message-ID.
#[derive(Debug, Clone)]
pub struct AskRef {
    pub address: String,
    pub message_id: String,
}

/// Why a reply was not taken, as the person's Inbox shows it (spec ENG-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedKind {
    NotOnList,
    Dmarc,
    Unclear,
    /// A robot, a bounce, kwant's own mail: not recorded.
    Ignored,
}

impl RefusedKind {
    pub fn of(reason: &str) -> Self {
        let r = reason.to_ascii_lowercase();
        if r.contains("allowlist") {
            RefusedKind::NotOnList
        } else if r.contains("dmarc") || r.contains("from") {
            // A From that can't be proven or isn't one address: treated as forged
            RefusedKind::Dmarc
        } else if r.starts_with("automated") || r == "bounce" || r == "our own mail" {
            RefusedKind::Ignored
        } else {
            RefusedKind::Unclear
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RefusedKind::NotOnList => "not_on_list",
            RefusedKind::Dmarc => "dmarc",
            RefusedKind::Unclear => "unclear",
            RefusedKind::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnswerOutcome {
    /// `asks[index]` is answered by `from` with `text` (top of the reply only).
    Answer {
        index: usize,
        from: String,
        text: String,
    },
    Refused {
        index: usize,
        from: Option<String>,
        reason: String,
        kind: RefusedKind,
    },
    NotOurs,
}

/// Which waiting question a mail answers, if any: by the address it was sent to, or by the
/// Message-ID it replies to. Only the send list, DMARC passed aligned, counts.
pub fn answer_for(m: &Polled, asks: &[AskRef], allow: &[String], own: &[String]) -> AnswerOutcome {
    let Some(p) = &m.parsed else {
        return AnswerOutcome::NotOurs;
    };
    let index = asks.iter().position(|a| {
        let addr = a.address.to_ascii_lowercase();
        let id = a.message_id.trim_matches(['<', '>']).to_ascii_lowercase();
        p.recipients.contains(&addr) || p.refers_to.contains(&id)
    });
    let Some(index) = index else {
        return AnswerOutcome::NotOurs;
    };
    match answer_in(&p.mail, allow, own) {
        Ok((from, text)) => AnswerOutcome::Answer { index, from, text },
        Err(reason) => {
            let from = single_from(&p.mail).from.map(|f| base_address(&f));
            AnswerOutcome::Refused {
                index,
                from,
                kind: RefusedKind::of(&reason),
                reason,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! A provider that is not IMAP (addresses assigned by the service, mail over "HTTPS") goes
    //! through the same checks: a forged From is still refused (phase 3 fit, Ada Nov 11).
    use super::*;
    use crate::mail::{OutgoingMail, RawMail};
    use std::sync::Mutex;

    struct Hosted {
        inbox: Mutex<Vec<RawMail>>,
    }

    #[async_trait::async_trait]
    impl MailProvider for Hosted {
        fn kind(&self) -> &'static str {
            "hosted_test"
        }
        fn own_addresses(&self) -> Vec<String> {
            vec!["u7x2q@bots.hosted.example".into()]
        }
        fn tagged(&self) -> bool {
            true
        }
        fn trusted_authserv(&self) -> Vec<String> {
            vec!["mx.hosted.example".into()]
        }
        async fn new_address(&self, _tag_hint: &str) -> Result<String, MailError> {
            // The service assigns the address; kwant's tag isn't in it
            Ok("u7x2q@bots.hosted.example".into())
        }
        async fn fetch(&self, _q: &FetchQuery) -> Result<Vec<RawMail>, MailError> {
            Ok(self.inbox.lock().unwrap().clone())
        }
        async fn send(&self, _m: &OutgoingMail) -> Result<(), MailError> {
            Ok(())
        }
        async fn check(&self) -> Result<(), MailError> {
            Ok(())
        }
    }

    fn raw(key: &str, auth: &str, from: &str, to: &str, extra: &str, body: &str) -> RawMail {
        let bytes = format!(
            "Authentication-Results: {auth}\r\nFrom: {from}\r\nTo: {to}\r\nSubject: Your code\r\n\
             Message-ID: <{key}@sender.example>\r\n{extra}Content-Type: text/plain\r\n\r\n{body}\r\n"
        );
        RawMail {
            key: key.into(),
            bytes: bytes.into_bytes(),
            received: Some(Utc::now()),
        }
    }

    #[tokio::test]
    async fn a_forged_from_through_a_hosted_provider_is_still_refused() {
        let addr = "u7x2q@bots.hosted.example";
        let ask = AskRef {
            address: addr.into(),
            message_id: "q_1.abcde@kwant".into(),
        };
        let allow = vec!["owner@example.org".to_string()];
        let hosted = Hosted {
            inbox: Mutex::new(vec![
                // Claims to be the owner; the trusted server saw DMARC fail
                raw(
                    "forged",
                    "mx.hosted.example; dmarc=fail header.from=example.org",
                    "Owner <owner@example.org>",
                    addr,
                    "In-Reply-To: <q_1.abcde@kwant>\r\n",
                    "1",
                ),
                // Claims a pass, but from a server the provider doesn't trust
                raw(
                    "untrusted",
                    "mx.evil.example; dmarc=pass header.from=example.org",
                    "owner@example.org",
                    addr,
                    "In-Reply-To: <q_1.abcde@kwant>\r\n",
                    "1",
                ),
                // Two From addresses
                raw(
                    "two",
                    "mx.hosted.example; dmarc=pass header.from=example.org",
                    "owner@example.org, mallory@evil.example",
                    addr,
                    "",
                    "1",
                ),
                // The real owner
                raw(
                    "real",
                    "mx.hosted.example; dmarc=pass header.from=example.org",
                    "Owner <owner@example.org>",
                    addr,
                    "In-Reply-To: <q_1.abcde@kwant>\r\n",
                    "2\r\n\r\nOn Tue, kwant wrote:\r\n> Which one?",
                ),
                // A code from a site, with a forged From domain
                raw(
                    "code-forged",
                    "mx.hosted.example; dmarc=fail header.from=tiktok.com",
                    "TikTok <no-reply@tiktok.com>",
                    addr,
                    "",
                    "Your code is 482913",
                ),
                raw(
                    "code",
                    "mx.hosted.example; dmarc=pass header.from=tiktok.com",
                    "TikTok <no-reply@tiktok.com>",
                    addr,
                    "",
                    "Your code is 482913",
                ),
            ]),
        };
        assert_eq!(
            hosted.new_address("kwant-drafter-k7q2m").await.unwrap(),
            addr
        );
        let polled = poll(
            &hosted,
            Some(vec![addr.into()]),
            Utc::now(),
            &HashSet::new(),
        )
        .await
        .unwrap();
        let by = |k: &str| polled.iter().find(|m| m.key == k).unwrap();
        let own = hosted.own_addresses();
        for k in ["forged", "untrusted", "two"] {
            match answer_for(by(k), std::slice::from_ref(&ask), &allow, &own) {
                AnswerOutcome::Refused { kind, .. } => assert_eq!(kind, RefusedKind::Dmarc, "{k}"),
                other => panic!("{k}: {other:?}"),
            }
        }
        assert_eq!(
            answer_for(by("real"), std::slice::from_ref(&ask), &allow, &own),
            AnswerOutcome::Answer {
                index: 0,
                from: "owner@example.org".into(),
                text: "2".into()
            }
        );
        assert!(
            matches!(code_for(by("code-forged"), addr, true, Some("tiktok.com")), CodeOutcome::Refused(r) if r.contains("DMARC"))
        );
        match code_for(by("code"), addr, true, Some("tiktok.com")) {
            CodeOutcome::Code(f) => assert_eq!(f.code.as_deref(), Some("482913")),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn mail_before_the_wait_and_mail_to_other_addresses_is_not_this_waits() {
        let addr = "bots+kwant-a-b2c3d@example.com";
        let mut old = raw(
            "old",
            "mx.hosted.example; dmarc=pass header.from=site.example",
            "no-reply@site.example",
            addr,
            "",
            "Code: 111222",
        );
        old.received = Some(Utc::now() - Duration::hours(1));
        let other = raw(
            "other",
            "mx.hosted.example; dmarc=pass header.from=site.example",
            "no-reply@site.example",
            "bots+kwant-b-x2y3z@example.com",
            "",
            "Code: 333444",
        );
        let hosted = Hosted {
            inbox: Mutex::new(vec![old, other]),
        };
        let polled = poll(&hosted, None, Utc::now(), &HashSet::new())
            .await
            .unwrap();
        assert_eq!(polled.len(), 1);
        assert_eq!(code_for(&polled[0], addr, true, None), CodeOutcome::NotOurs);
        // A shared address: another site's mail is left for its own wait
        assert_eq!(
            code_for(&polled[0], "bots@example.com", false, Some("tiktok.com")),
            CodeOutcome::NotOurs
        );
        let skip: HashSet<String> = ["other".to_string()].into();
        assert!(poll(&hosted, None, Utc::now(), &skip)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sites_and_refusal_kinds() {
        assert_eq!(
            site_domain("https://www.TikTok.com/signup?x=1").as_deref(),
            Some("tiktok.com")
        );
        assert_eq!(
            site_domain("accounts.google.com").as_deref(),
            Some("accounts.google.com")
        );
        assert_eq!(site_domain("localhost"), None);
        assert_eq!(site_domain("a b.com"), None);
        assert_eq!(
            RefusedKind::of("sender not on allowlist"),
            RefusedKind::NotOnList
        );
        assert_eq!(
            RefusedKind::of("DMARC did not pass aligned"),
            RefusedKind::Dmarc
        );
        assert_eq!(
            RefusedKind::of("automated mail (another kwant)"),
            RefusedKind::Ignored
        );
        assert_eq!(RefusedKind::of("empty reply"), RefusedKind::Unclear);
    }
}
