//! What kwant does with one received mail, as pure decisions: a code for a waiting bot, or an
//! answer to a waiting question, or a reason to skip it. The server adds the tag, the caps and
//! the database; these rules are the same for every mail provider.

use super::safety::{
    answer_text, automation, base_address, check_sender, domain_of, extract_code, is_bounce,
    single_from, Allow, Found, Mail, AGENT_MAIL,
};

/// The code or sign-in link in a mail to a sign-up address, and nothing else of it. Anyone may
/// send a code, but DMARC must pass aligned for its From, and when the bot named the site, the
/// From must be that site's domain (or under it) and a link must point there.
pub fn code_in(mail: &Mail, site: Option<&str>) -> Result<Found, String> {
    if is_bounce(mail) {
        return Err("bounce".into());
    }
    let v = check_sender(mail, &Allow::Anyone);
    if !v.ok {
        return Err(v.reason.unwrap_or_default());
    }
    if let Some(site) = site {
        let d = domain_of(&v.from.unwrap_or_default());
        if d != site && !d.ends_with(&format!(".{site}")) {
            return Err(format!("not from {site}"));
        }
    }
    let found = extract_code(&mail.subject, &mail.text, site);
    if found.code.is_none() && found.link.is_none() {
        return Err("no code or link".into());
    }
    Ok(found)
}

/// The person's answer in a reply: `(from, answer)`. Only from the send list, with DMARC passed
/// aligned, never from kwant's own addresses or from a robot, and only the top of the reply,
/// without the quote and signature.
pub fn answer_in(
    mail: &Mail,
    allow: &[String],
    own: &[String],
) -> Result<(String, String), String> {
    if is_bounce(mail) {
        return Err("bounce".into());
    }
    let single = single_from(mail);
    if let Some(from) = single.from.as_deref() {
        let me = base_address(from);
        if own.iter().any(|o| base_address(o) == me) {
            return Err("our own mail".into());
        }
    }
    match automation(mail) {
        None => {}
        // kwant differs from ClankerInbox here: a person's reply never carries kwant's own loop
        // marks, so a mail that does is another kwant (or this one's own mail coming back)
        Some(a) if a == AGENT_MAIL => return Err("automated mail (another kwant)".into()),
        Some(reason) => return Err(reason),
    }
    let v = check_sender(mail, &Allow::List(allow.to_vec()));
    if !v.ok {
        return Err(v.reason.unwrap_or_default());
    }
    let answer = answer_text(&mail.text, mail.html.as_deref());
    if answer.is_empty() {
        return Err("empty reply".into());
    }
    Ok((v.from.unwrap_or_default(), answer))
}

#[cfg(test)]
mod tests {
    //! The scenarios of ClankerInbox's `test/shared-safety.test.ts` that are about one mail.
    use super::super::safety::{Dmarc, LOOP_MARK};
    use super::*;

    const OWNER: &str = "owner@example.org";
    const STRANGER: &str = "mallory@evil.example";
    const SERVICE: &str = "no-reply@service.example";
    const BOT: &str = "bots@inbox.example.dev";

    fn mail(from: &str, text: &str) -> Mail {
        Mail {
            headers: vec![("from".into(), from.into())],
            from: Some(from.into()),
            mail_from: Some(from.into()),
            dmarc: Some(Dmarc {
                result: "pass".into(),
                aligned: true,
            }),
            subject: "Re: Ship it?".into(),
            text: text.into(),
            html: None,
        }
    }

    fn auth(m: Mail, how: &str) -> Mail {
        let dmarc = match how {
            "unaligned" => Some(Dmarc {
                result: "pass".into(),
                aligned: false,
            }),
            "fail" => Some(Dmarc {
                result: "fail".into(),
                aligned: true,
            }),
            _ => None,
        };
        Mail { dmarc, ..m }
    }

    fn with(m: Mail, h: &[(&str, &str)]) -> Mail {
        let mut m = m;
        m.headers.extend(
            h.iter()
                .map(|(n, v)| (n.to_ascii_lowercase(), v.to_string())),
        );
        m
    }

    fn allow() -> Vec<String> {
        vec![OWNER.into()]
    }

    fn own() -> Vec<String> {
        vec![BOT.into()]
    }

    #[test]
    fn a_strangers_reply_is_no_answer_and_an_empty_send_list_lets_nobody_in() {
        assert_eq!(
            answer_in(&mail(STRANGER, "1"), &allow(), &own()).unwrap_err(),
            "sender not on allowlist"
        );
        assert_eq!(
            answer_in(&mail(OWNER, "1"), &[], &own()).unwrap_err(),
            "allowlist is empty"
        );
        assert_eq!(
            answer_in(&mail(OWNER, "1"), &allow(), &own()).unwrap(),
            (OWNER.to_string(), "1".to_string())
        );
    }

    #[test]
    fn dmarc_must_pass_aligned_unaligned_fail_and_missing_results_are_no_answer() {
        for how in ["unaligned", "fail", "none"] {
            assert_eq!(
                answer_in(&auth(mail(OWNER, "1"), how), &allow(), &own()).unwrap_err(),
                "DMARC did not pass aligned",
                "{how}"
            );
        }
    }

    #[test]
    fn auto_replies_lists_bulk_and_bounces_are_no_answer() {
        let marks: &[&[(&str, &str)]] = &[
            &[("Auto-Submitted", "auto-replied")],
            &[("Auto-Submitted", "auto-notified")],
            &[("X-Loop", "someone")],
            &[("X-Loop", LOOP_MARK), ("Auto-Submitted", "auto-replied")],
            &[("Precedence", "bulk")],
            &[("Precedence", "bulk; x=1")],
            &[("Precedence", "junk")],
            &[("Precedence", "list")],
            &[("X-Autoreply", "yes")],
            &[("X-Autorespond", "yes")],
            &[("List-Id", "<team.lists.example.org>")],
            &[("List-Unsubscribe", "<mailto:leave@example.org>")],
        ];
        for h in marks {
            let e = answer_in(&with(mail(OWNER, "1"), h), &allow(), &own()).unwrap_err();
            assert!(e.starts_with("automated mail"), "{h:?}: {e}");
        }
        let bounce = Mail {
            mail_from: Some(String::new()),
            ..mail(OWNER, "1")
        };
        assert_eq!(answer_in(&bounce, &allow(), &own()).unwrap_err(), "bounce");
        // kwant: the marks kwant itself sends are another kwant's, not the person's
        let agent = with(
            mail(OWNER, "2"),
            &[("Auto-Submitted", "auto-generated"), ("X-Loop", LOOP_MARK)],
        );
        assert_eq!(
            answer_in(&agent, &allow(), &own()).unwrap_err(),
            "automated mail (another kwant)"
        );
    }

    #[test]
    fn kwants_own_mail_is_never_an_answer() {
        // Even when the bots' address is on the send list (a mailbox that writes to itself)
        let both = vec![OWNER.to_string(), BOT.to_string()];
        assert_eq!(
            answer_in(
                &mail("bots+kwant-drafter-k7q2m@inbox.example.dev", "1"),
                &both,
                &own()
            )
            .unwrap_err(),
            "our own mail"
        );
    }

    #[test]
    fn quoted_text_is_stripped_so_the_quoted_options_are_not_the_answer() {
        let text = [
            "2",
            "",
            "On Tue, 10 Nov 2026 at 10:00, Bot <bot@inbox.example.dev> wrote:",
            "> Ship it?",
            ">",
            "> 1. Yes",
            "> 2. No",
            "",
        ]
        .join("\n");
        let (_, answer) = answer_in(&mail(OWNER, &text), &allow(), &own()).unwrap();
        assert_eq!(answer, "2");
        assert_eq!(
            super::super::safety::match_option(&answer, &["Yes".into(), "No".into()]).as_deref(),
            Some("No")
        );
        assert_eq!(
            answer_in(&mail(OWNER, "> only the quote"), &allow(), &own()).unwrap_err(),
            "empty reply"
        );
    }

    #[test]
    fn codes_only_the_code_leaves_only_from_the_site_with_dmarc_pass() {
        let body = "Hi! Your verification code is 482913. Private note: account 99887766 is yours.";
        let phish = Mail {
            subject: "Your code".into(),
            ..mail("no-reply@phish.example", "Your code is 111111")
        };
        assert_eq!(
            code_in(&phish, Some("service.example")).unwrap_err(),
            "not from service.example"
        );
        let forged = Mail {
            subject: "Your code".into(),
            ..auth(mail(SERVICE, "Your code is 222222"), "fail")
        };
        assert_eq!(
            code_in(&forged, Some("service.example")).unwrap_err(),
            "DMARC did not pass aligned"
        );
        let real = Mail {
            subject: "Your code".into(),
            ..mail(SERVICE, body)
        };
        assert_eq!(
            code_in(&real, Some("service.example")).unwrap(),
            Found {
                code: Some("482913".into()),
                link: None
            }
        );
        // A subdomain of the site counts; a look-alike doesn't
        let sub = Mail {
            subject: "Your code".into(),
            ..mail("no-reply@mail.service.example", "Your code is 482913")
        };
        assert!(code_in(&sub, Some("service.example")).is_ok());
        let alike = Mail {
            subject: "Your code".into(),
            ..mail("no-reply@notservice.example", "Your code is 482913")
        };
        assert!(code_in(&alike, Some("service.example")).is_err());
        // Without a site: any sender that passed DMARC
        assert!(code_in(&phish, None).is_ok());
        assert_eq!(
            code_in(
                &Mail {
                    subject: "Welcome".into(),
                    ..mail(SERVICE, "Thanks for joining")
                },
                None
            )
            .unwrap_err(),
            "no code or link"
        );
        let bounce = Mail {
            mail_from: Some("<>".into()),
            ..real
        };
        assert_eq!(code_in(&bounce, None).unwrap_err(), "bounce");
    }
}
