//! Ported from ClankerInbox (MIT, Danny Postma): `test/safety.test.ts` and `test/replies.test.ts`
//! at 8c8cf3be. The case tables are copied as they are; where kwant deliberately differs, the test
//! says so (the loop mark is `kwant`, and a mail with no Return-Path is not a bounce over IMAP).

use super::*;

const OWNER: &str = "owner+clankerinbox@example.org";

/// The reference's `msg()`: a reply from the owner that passed DMARC aligned.
fn msg() -> Mail {
    Mail {
        headers: vec![("from".into(), "Owner <owner@example.org>".into())],
        from: Some("owner@example.org".into()),
        mail_from: Some("owner@example.org".into()),
        dmarc: Some(Dmarc {
            result: "pass".into(),
            aligned: true,
        }),
        subject: "Re: q".into(),
        text: "yes".into(),
        html: None,
    }
}

fn with_headers(h: &[(&str, &str)]) -> Mail {
    Mail {
        headers: h
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
        ..msg()
    }
}

fn list(a: &[&str]) -> Allow {
    Allow::List(a.iter().map(|s| s.to_string()).collect())
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn found(code: Option<&str>, link: Option<&str>) -> Found {
    Found {
        code: code.map(Into::into),
        link: link.map(Into::into),
    }
}

#[test]
fn receive_default_deny_when_the_allowlist_is_empty() {
    let v = check_sender(&msg(), &Allow::List(vec![]));
    assert!(!v.ok);
    assert!(v.reason.unwrap().contains("empty"));
}

#[test]
fn receive_allowlisted_sender_with_aligned_dmarc_pass_is_accepted_plus_tags_are_the_same_mailbox() {
    let v = check_sender(&msg(), &list(&[OWNER]));
    assert!(v.ok, "{v:?}");
    assert_eq!(v.from.as_deref(), Some("owner@example.org"));
}

#[test]
fn receive_sender_not_on_the_allowlist_is_rejected() {
    let m = Mail {
        from: Some("eve@example.org".into()),
        ..with_headers(&[("from", "eve@example.org")])
    };
    let v = check_sender(&m, &list(&[OWNER]));
    assert!(!v.ok);
    assert!(v.reason.unwrap().contains("allowlist"));
}

#[test]
fn receive_from_must_be_exactly_one_address() {
    let none = Mail {
        from: None,
        ..with_headers(&[])
    };
    assert_eq!(
        check_sender(&none, &list(&[OWNER])).reason.as_deref(),
        Some("no From address")
    );
    let two = with_headers(&[("from", "owner@example.org, eve@evil.com")]);
    assert_eq!(
        check_sender(&two, &list(&[OWNER])).reason.as_deref(),
        Some("more than one From address")
    );
    let differ = with_headers(&[("from", "eve@evil.com")]);
    assert!(!check_sender(&differ, &list(&[OWNER])).ok);
    // A quoted display name that looks like an address is not a second address
    assert_eq!(
        parse_addresses("\"eve@evil.com\" <owner@example.org>"),
        s(&["owner@example.org"])
    );
    assert!(
        single_from(&with_headers(&[(
            "from",
            "\"eve@evil.com\" <owner@example.org>"
        )]))
        .ok
    );
}

#[test]
fn receive_comments_and_quoted_strings_are_read_as_rfc_5322_reads_them() {
    assert!(parse_addresses("(owner@example.org \") attacker@evil.example (\")").is_empty());
    assert_eq!(
        parse_addresses("owner@example.org (Owner)"),
        s(&["owner@example.org"])
    );
    assert_eq!(
        parse_addresses("\"Doe, J (work)\" <owner@example.org>"),
        s(&["owner@example.org"])
    );
    assert_eq!(
        parse_addresses("Grp: owner@example.org;"),
        s(&["owner@example.org"])
    );
    let long = format!("{} <owner@example.org>", "x".repeat(5000));
    for unclear in [
        "(\") attacker@evil.example (\"owner@example.org) ",
        "\"a\\\" <owner@example.org> \\\"b\" <attacker@evil.example>",
        "(unclosed owner@example.org",
        "owner@example.org)",
        "\"unclosed <owner@example.org>",
        "<owner@example.org",
        "<<owner@example.org>>",
        "\"owner\"@example.org",
        "owner@example.org@evil.example",
        "<@evil.example:owner@example.org>",
        long.as_str(),
    ] {
        assert!(parse_addresses(unclear).is_empty(), "{unclear}");
    }
    let trick = with_headers(&[("from", "(owner@example.org \") attacker@evil.example (\")")]);
    assert!(!check_sender(&trick, &list(&[OWNER])).ok);
    // A From header that is there but unclear is never replaced by the provider's own field
    assert!(
        !check_sender(
            &with_headers(&[("from", "\"owner@example.org")]),
            &list(&[OWNER])
        )
        .ok
    );
    assert!(!check_sender(&with_headers(&[("from", "undisclosed")]), &list(&[OWNER])).ok);
}

#[test]
fn receive_dmarc_must_pass_and_be_aligned() {
    let d = |result: &str, aligned| Mail {
        dmarc: Some(Dmarc {
            result: result.into(),
            aligned,
        }),
        ..msg()
    };
    assert!(!check_sender(&d("fail", true), &list(&[OWNER])).ok);
    assert!(!check_sender(&d("pass", false), &list(&[OWNER])).ok);
    assert!(
        !check_sender(
            &Mail {
                dmarc: None,
                ..msg()
            },
            &list(&[OWNER])
        )
        .ok
    );
    assert!(
        !check_sender(
            &Mail {
                dmarc: None,
                ..msg()
            },
            &Allow::Anyone
        )
        .ok
    );
    assert!(check_sender(&msg(), &Allow::Anyone).ok);
}

#[test]
fn receive_bounces_are_recognised() {
    let daemon = Mail {
        from: Some("MAILER-DAEMON@mx.example.com".into()),
        ..with_headers(&[("from", "MAILER-DAEMON@mx.example.com")])
    };
    assert!(is_bounce(&daemon));
    assert!(is_bounce(&Mail {
        mail_from: Some(String::new()),
        ..msg()
    }));
    assert!(is_bounce(&Mail {
        mail_from: Some("<>".into()),
        ..msg()
    }));
    assert!(is_bounce(&with_headers(&[
        ("from", "owner@example.org"),
        (
            "content-type",
            "multipart/report; report-type=delivery-status"
        )
    ])));
    assert!(is_bounce(&with_headers(&[
        ("from", "owner@example.org"),
        ("x-failed-recipients", "a@b.com")
    ])));
    assert!(!is_bounce(&msg()));
    // kwant: an IMAP mail without a Return-Path says nothing about its envelope, so it is no
    // bounce by itself (the reference counts a missing envelope as one)
    assert!(!is_bounce(&Mail {
        mail_from: None,
        ..msg()
    }));
}

#[test]
fn receive_auto_submitted_and_x_loop_mail_is_automated() {
    let auto = |h: &[(&str, &str)]| {
        let mut all = vec![("from", "a@b.com")];
        all.extend_from_slice(h);
        automation(&with_headers(&all)).is_some()
    };
    assert!(auto(&[("auto-submitted", "auto-replied")]));
    assert!(auto(&[("x-loop", "kwant")]));
    assert!(auto(&[("precedence", "bulk")]));
    assert!(!auto(&[("auto-submitted", "no")]));
    assert!(automation(&msg()).is_none());
}

#[test]
fn receive_the_marks_of_another_kwant_are_told_apart_from_auto_replies_and_lists() {
    let kind = |h: &[(&str, &str)]| {
        let mut all = vec![("from", "a@b.com")];
        all.extend_from_slice(h);
        automation(&with_headers(&all))
    };
    let agent = Some(AGENT_MAIL.to_string());
    assert_eq!(kind(&[]), None);
    assert_eq!(kind(&[("auto-submitted", "no")]), None);
    assert_eq!(kind(&[("auto-submitted", "auto-generated")]), agent);
    assert_eq!(kind(&[("x-loop", "kwant")]), agent);
    assert_eq!(
        kind(&[("auto-submitted", "Auto-Generated"), ("x-loop", "KWANT")]),
        agent
    );
    assert_eq!(
        kind(&[
            ("auto-submitted", "auto-generated"),
            ("auto-submitted", "auto-generated"),
            ("x-loop", "kwant")
        ]),
        agent
    );
    assert_eq!(
        kind(&[("auto-submitted", "auto-generated; owner-email=\"x@b.com\"")]),
        agent
    );
    let has = |h: &[(&str, &str)], what: &str| {
        let k = kind(h).unwrap_or_default();
        assert!(k.contains(what), "{h:?}: {k}");
    };
    has(&[("auto-submitted", "auto-replied")], "auto-reply");
    has(
        &[("auto-submitted", "auto-generated, auto-replied")],
        "auto-reply",
    );
    has(&[("auto-submitted", "auto-notified")], "auto-reply");
    has(&[("x-loop", "helpdesk@b.com")], "X-Loop");
    has(&[("x-loop", "kwant"), ("x-loop", "other")], "X-Loop");
    has(&[("x-loop", " ")], "X-Loop");
    // The reference's own mark is another tool's to kwant
    has(&[("x-loop", "clankerinbox")], "X-Loop");
    has(
        &[("auto-submitted", "auto-generated"), ("precedence", "bulk")],
        "Precedence",
    );
    has(&[("precedence", "Bulk; x=1")], "Precedence");
    has(&[("precedence", "list")], "Precedence");
    has(&[("precedence", "auto_reply")], "Precedence");
    assert_eq!(kind(&[("precedence", "first-class")]), None);
    has(
        &[("x-autoreply", "yes"), ("auto-submitted", "auto-generated")],
        "auto-reply",
    );
    has(&[("x-autorespond", "yes")], "auto-reply");
    has(&[("list-id", "<team.example.org>")], "mailing list");
    has(
        &[
            ("list-unsubscribe", "<mailto:x@b.com>"),
            ("x-loop", "kwant"),
        ],
        "mailing list",
    );
}

#[test]
fn send_the_send_list_refuses_any_other_recipient_own_addresses_are_allowed() {
    let send = s(&[OWNER]);
    let own = s(&["ask@inbox.example.dev", "signup@inbox.example.dev"]);
    assert!(check_recipients(&s(&[OWNER]), &send, &own).ok);
    assert!(check_recipients(&s(&["Owner <owner+clankerinbox@example.org>"]), &send, &own).ok);
    assert!(check_recipients(&s(&["signup+t1@inbox.example.dev"]), &send, &own).ok);
    assert!(!check_recipients(&s(&["eve@evil.com"]), &send, &own).ok);
    assert!(!check_recipients(&s(&[OWNER, "eve@evil.com"]), &send, &own).ok);
    assert!(!check_recipients(&s(&["owner@example.org, eve@evil.com"]), &send, &own).ok);
    assert!(!check_recipients(&[], &send, &own).ok);
    assert!(!check_recipients(&s(&[OWNER]), &[], &own).ok);
}

#[test]
fn quoted_text_is_stripped_before_the_answer_is_read() {
    let gmail = "Option B please\n\nOn Mon, 9 Nov 2026 at 15:02, Clanker Inbox <clankerinbox@inbox.example.dev> wrote:\n> Which one?\n> 1. Option A\n> 2. Option B\n";
    assert_eq!(strip_quoted(gmail), "Option B please");
    let wrapped = "yes\n\nOn Mon, 9 Nov 2026 at 15:02, Clanker Inbox <\nclankerinbox@inbox.example.dev> wrote:\n> q";
    assert_eq!(strip_quoted(wrapped), "yes");
    let outlook = "go ahead\r\n\r\nFrom: Clanker Inbox <a@b.com>\r\nSent: Monday\r\nTo: Owner\r\nSubject: q\r\n\r\nWhich one?";
    assert_eq!(strip_quoted(outlook), "go ahead");
    assert_eq!(strip_quoted("ok\n-- \nOwner"), "ok");
    assert_eq!(strip_quoted("ok\n\nSent from my iPhone"), "ok");
    assert_eq!(strip_quoted("> only a quote"), "");
    assert_eq!(strip_quoted(""), "");
}

#[test]
fn answers_match_options_by_number_or_text_else_stay_free_text() {
    let opts = s(&["Ship it", "Wait"]);
    assert_eq!(match_option("2", &opts).as_deref(), Some("Wait"));
    assert_eq!(match_option("ship it.", &opts).as_deref(), Some("Ship it"));
    assert_eq!(
        match_option("Wait\nbecause reasons", &opts).as_deref(),
        Some("Wait")
    );
    assert_eq!(match_option("3", &opts), None);
    assert_eq!(match_option("maybe later", &opts), None);
    assert_eq!(match_option("", &opts), None);
}

#[test]
fn codes_a_4_to_8_digit_code_is_extracted_preferring_one_near_code_words() {
    assert_eq!(
        extract_code("Your verification code is 482913", "", None),
        found(Some("482913"), None)
    );
    assert_eq!(
        extract_code("Welcome", "Order 2026 shipped. Your code: 7731", None),
        found(Some("7731"), None)
    );
    assert_eq!(
        extract_code("Sign in", "Use 123-456 to log in", None),
        found(Some("123456"), None)
    );
    assert_eq!(
        extract_code("Hi", "no digits here 12 or 123456789", None),
        Found::default()
    );
}

#[test]
fn codes_a_sign_in_link_is_returned_when_there_is_no_code() {
    assert_eq!(
        extract_code(
            "Log in to Tool",
            "Click https://tool.example.com/auth/magic?token=abc. Thanks",
            None
        ),
        found(None, Some("https://tool.example.com/auth/magic?token=abc"))
    );
    assert_eq!(
        extract_code("News", "Read https://tool.example.com/blog", None),
        Found::default()
    );
    // A footer ZIP code does not beat the sign-in link
    assert_eq!(
        extract_code(
            "Your sign-in link",
            "Open https://tool.example.com/magic/abc\n\nTool Inc, San Francisco, CA 94107",
            None
        ),
        found(None, Some("https://tool.example.com/magic/abc"))
    );
}

#[test]
fn the_mail_log_masks_codes_and_link_secrets() {
    assert_eq!(mask_secrets("code 482913"), "code ••••••");
    assert_eq!(
        mask_secrets("code 482-913 or 482 913"),
        "code •••-••• or ••• •••"
    );
    assert_eq!(
        mask_secrets("open https://x.dev/verify?t=abc now"),
        "open https://x.dev/… now"
    );
    assert_eq!(
        mask_secrets("open https://x.dev/magic/Zm9vYmFy, then"),
        "open https://x.dev/…, then"
    );
    assert_eq!(
        mask_secrets("see https://x.dev/ or https://x.dev"),
        "see https://x.dev/ or https://x.dev"
    );
    assert_eq!(mask_secrets("https://x.dev?t=abc"), "https://x.dev/…");
    assert_eq!(
        mask_secrets("option 2, call +1 415 555 0123 on 2026-11-10"),
        "option 2, call +1 415 555 0123 on 2026-11-10"
    );
}

#[test]
fn control_sequences_from_mail_are_dropped_and_names_stay_plain() {
    assert_eq!(clean_text("ok\u{1b}[2K\u{1b}[1A\r✓ done"), "ok✓ done");
    assert_eq!(
        clean_text("a\u{1b}]8;;https://evil.example\u{7}link\u{1b}]8;;\u{1b}\\b"),
        "alinkb"
    );
    assert_eq!(clean_text("a\u{9b}31mb\u{85}c\u{7}d\u{202e}e"), "abcde");
    assert_eq!(clean_text("line 1\nline 2\ttab"), "line 1\nline 2\ttab");
    assert_eq!(clean_line("Subject\nBcc: x\u{1b}[31m"), "Subject Bcc: x");
    assert_eq!(
        safe_filename("../../.ssh/authorized_keys"),
        ".._.._.ssh_authorized_keys"
    );
    assert_eq!(safe_filename("a\u{1b}[31m\r\nb.txt"), "a b.txt");
    assert_eq!(safe_filename("C:\\Windows\\x.dll"), "C:_Windows_x.dll");
    for bad in ["..", ".", "", "\u{1b}"] {
        assert_eq!(safe_filename(bad), "attachment", "{bad:?}");
    }
    assert!(has_control_chars("a\r\nBcc: x"));
    assert!(has_control_chars("a\u{2028}b"));
    assert!(!has_control_chars("Which day? Monday – or later"));
}

#[test]
fn codes_no_digits_out_of_phone_numbers_dates_or_longer_runs() {
    assert_eq!(
        extract_code(
            "Verify your email",
            "Click the button in the app to verify. Questions? Call us at +1 415 555 0123.",
            None
        ),
        Found::default()
    );
    assert_eq!(
        extract_code(
            "Hi",
            "Call (415) 555-0123 or 415.555.0123 or 415 555 0123",
            None
        ),
        Found::default()
    );
    assert_eq!(
        extract_code(
            "Your code",
            "On 10/11/2026 at 10:00. Your one-time code: 4821",
            None
        ),
        found(Some("4821"), None)
    );
    assert_eq!(
        extract_code(
            "Your code",
            "Reference 2026-11-10-4471. Your code is 482 913",
            None
        ),
        found(Some("482913"), None)
    );
    assert_eq!(
        extract_code(
            "Hi",
            "Order 55512 shipped.\n\nYour verification code is 9921.",
            None
        ),
        found(Some("9921"), None)
    );
    assert_eq!(
        extract_code("Your code", "code 31337420", None),
        found(Some("31337420"), None)
    );
    assert_eq!(
        extract_code("Your code", "Your PIN: 12345678901", None),
        Found::default()
    );
}

#[test]
fn codes_a_link_is_https_to_a_public_host_and_to_the_expected_domain() {
    for bad in [
        "Sign in: https://127.0.0.1:8080/admin/reset?confirm=1",
        "Sign in: https://0x7f.1/login",
        "Sign in: https://[::1]/login",
        "Sign in: https://localhost/login",
        "Sign in: https://intranet/login",
        "Sign in: https://db.internal/login",
        "Sign in: https://printer.local/login",
        "Sign in: https://github.com@evil.example/login",
        "Sign in: http://service.example/login",
    ] {
        assert_eq!(
            extract_code("Sign in", bad, None),
            Found::default(),
            "{bad}"
        );
    }
    let mail = "Log in: https://login.service.example/magic?t=1 or https://evil.example/login?t=2";
    assert_eq!(
        extract_code("Log in", mail, None),
        found(None, Some("https://login.service.example/magic?t=1"))
    );
    assert_eq!(
        extract_code("Log in", mail, Some("service.example")),
        found(None, Some("https://login.service.example/magic?t=1"))
    );
    assert_eq!(
        extract_code(
            "Log in",
            "https://evil.example/login?t=2",
            Some("service.example")
        ),
        Found::default()
    );
    assert_eq!(
        extract_code(
            "Log in",
            "https://notservice.example/login",
            Some("service.example")
        ),
        Found::default()
    );
}

#[test]
fn codes_from_takes_a_domain_an_address_or_a_url() {
    for v in [
        "service.example",
        "@service.example",
        "no-reply@service.example",
        "SERVICE.EXAMPLE.",
        "https://service.example/x",
        " service.example ",
    ] {
        assert_eq!(sender_domain(v).as_deref(), Ok("service.example"), "{v}");
    }
    for v in ["", "not a domain", "https://", "a@b@c", "localhost"] {
        assert!(
            sender_domain(v).unwrap_err().contains("invalid from"),
            "{v}"
        );
    }
}

#[test]
fn options_exact_text_beats_an_index_an_index_is_plain_digits_in_range() {
    assert_eq!(
        match_option("1", &s(&["4", "2", "1"])).as_deref(),
        Some("1")
    );
    assert_eq!(match_option("2", &s(&["3", "2"])).as_deref(), Some("2"));
    assert_eq!(match_option("10", &s(&["5", "10"])).as_deref(), Some("10"));
    assert_eq!(match_option("3", &s(&["5", "10"])), None);
    let opts = s(&["Yes, ship it", "Wait until Monday", "Cancel"]);
    for (reply, want) in [
        ("1)", "Yes, ship it"),
        ("1.", "Yes, ship it"),
        ("#1", "Yes, ship it"),
        ("option 1", "Yes, ship it"),
        ("Option #2", "Wait until Monday"),
        ("  2  ", "Wait until Monday"),
        ("２", "Wait until Monday"),
        ("\u{200b}2", "Wait until Monday"),
        ("yes", "Yes, ship it"),
        ("wait", "Wait until Monday"),
        ("yes, ship it", "Yes, ship it"),
        ("Yes, ship it.", "Yes, ship it"),
        ("wait until monday!", "Wait until Monday"),
        ("\"Cancel\"", "Cancel"),
        ("*Cancel*", "Cancel"),
        ("Cancel.\n\nThanks,\nOwner", "Cancel"),
        ("Thanks!\n2", "Wait until Monday"),
        ("Hi,\n\n2\n\nOwner", "Wait until Monday"),
    ] {
        assert_eq!(
            match_option(reply, &opts).as_deref(),
            Some(want),
            "{reply:?}"
        );
    }
    for reply in [
        "4",
        "0",
        "0x2",
        "+2",
        "1e0",
        "1.0",
        "-1",
        "3 because the build failed",
        "10 minutes",
        "the first one",
        "Cancel?",
        "w",
        "👍",
        "٢",
        "maybe later",
        "",
        "Thanks!\nmaybe",
        "Sure thing, but first:\n2",
    ] {
        assert_eq!(match_option(reply, &opts), None, "{reply:?}");
    }
    // A prefix that fits two options picks neither
    assert_eq!(
        match_option("ship", &s(&["Ship it now", "Ship it Monday"])),
        None
    );
    assert_eq!(
        match_option("Ship it!", &s(&["Ship it!", "Wait"])).as_deref(),
        Some("Ship it!")
    );
}

// --- Reply styles (test/replies.test.ts) -------------------------------------------------------

const ASK: &str = "bot@inbox.example.dev";
const R_OWNER: &str = "owner@example.org";

fn qt(prefix: &str) -> String {
    [
        "Ship the release?",
        "",
        "Reply with a number or your own answer:",
        "",
        "1. Yes, ship it",
        "2. Wait until Monday",
        "3. Cancel",
    ]
    .iter()
    .map(|l| format!("{prefix}{l}"))
    .collect::<Vec<_>>()
    .join("\n")
}

const GMAIL_TEXT_LINES: &[&str] = &[
    "1",
    "",
    "",
    "Owner Name",
    "Founder at ExampleCo",
    "[image: LinkedIn] <https://www.card.example/x/linkedin> [image:",
    "X] <https://www.card.example/x/x> [image: Reviews]",
    "<https://www.card.example/x/reviews>",
    "[image: QR code for my digital business card]",
    "<https://www.card.example/x/qr>",
    "Scan to save",
    "my contact",
    "[image: Create your free email signature: Try it now]",
    "<https://www.card.example/x>",
    "",
    "On Wed, 11 Nov 2026 at 11:37, Clanker Inbox <",
    "clankerinbox@inbox.example.dev> wrote:",
    "",
    "> Reply with a number or your own answer:",
    ">",
    "> 1. It works",
    "> 2. Something is off",
    ">",
    "",
];
const GMAIL_SIGNATURE: &str = concat!(
    "<div dir=\"ltr\" class=\"gmail_signature\" data-smartmail=\"gmail_signature\"><div dir=\"ltr\"><table cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tbody>",
    "<tr><td><img src=\"https://cdn.card.example/a.png\" alt=\"\" width=\"80\" height=\"80\"></td><td><table><tbody>",
    "<tr><td style=\"font-size:17px;font-weight:700\">Owner Name</td></tr><tr><td style=\"font-size:13px\">Founder at ExampleCo</td></tr>",
    "<tr><td><a href=\"https://www.card.example/x/linkedin\" target=\"_blank\"><img src=\"https://cdn.card.example/li.png\" alt=\"LinkedIn\"></a></td></tr>",
    "</tbody></table></td><td align=\"right\"><a href=\"https://www.card.example/x/qr\"><img alt=\"QR code for my digital business card\"></a>",
    "<div>Scan to save<br>my contact</div></td></tr></tbody></table></div></div>",
);
const GMAIL_QUOTE: &str = concat!(
    "<div class=\"gmail_quote gmail_quote_container\"><div dir=\"ltr\" class=\"gmail_attr\">On Wed, 11 Nov 2026 at 11:37, Clanker Inbox &lt;<a href=\"mailto:clankerinbox@inbox.example.dev\">clankerinbox@inbox.example.dev</a>&gt; wrote:<br></div>",
    "<blockquote class=\"gmail_quote\" style=\"margin:0px 0px 0px 0.8ex;border-left-width:1px;border-left-style:solid;padding-left:1ex\">Reply with a number or your own answer:<br>\n<br>\n1. It works<br>\n2. Something is off<br>\n</blockquote></div>",
);

/// The case tables keep the reference's `${…}` parts as `{NAME}`; this fills them in.
fn expand(t: &str) -> String {
    let gmail_html = format!("<div dir=\"auto\">1</div><div><br clear=\"all\"><br clear=\"all\"><div>{GMAIL_SIGNATURE}</div></div><div><br>{GMAIL_QUOTE}</div>\n");
    t.replace("{ASK}", ASK)
        .replace("{OWNER}", R_OWNER)
        .replace("{QT}", &qt("> "))
        .replace("{QT0}", &qt(""))
        .replace("{SIG}", GMAIL_SIGNATURE)
        .replace("{QUOTE}", GMAIL_QUOTE)
        .replace("{GMAIL_HTML}", &gmail_html)
        .replace("{GMAIL_TEXT}", &GMAIL_TEXT_LINES.join("\n"))
}

// (name, reply text, expected answer, expected option)
const CASES: &[(&str, &str, &str, Option<&str>)] = &[
    ("gmail web", "1\n\nOn Tue, Nov 10, 2026 at 10:00 AM Bot <{ASK}> wrote:\n\n{QT}\n", "1", Some("Yes, ship it")),
    ("gmail android", "2\n\nOn Tue, 10 Nov 2026, 10:00 Bot, <{ASK}> wrote:\n\n{QT}\n", "2", Some("Wait until Monday")),
    ("apple mail", "Yes, ship it\n\n> On 10 Nov 2026, at 10:00, Bot <{ASK}> wrote:\n> \n{QT}\n", "Yes, ship it", Some("Yes, ship it")),
    ("ios mail sent from", "Cancel\n\nSent from my iPhone\n\n> On 10 Nov 2026, at 10:00, Bot <{ASK}> wrote:\n> \n{QT}\n", "Cancel", Some("Cancel")),
    ("ios mail top-only sig", "3\nSent from my iPhone", "3", Some("Cancel")),
    ("outlook desktop", "1\r\n\r\n-----Original Message-----\r\nFrom: Bot <{ASK}>\r\nSent: Tuesday, November 10, 2026 10:00 AM\r\nTo: Owner <{OWNER}>\r\nSubject: Ship the release?\r\n\r\n{QT0}", "1", Some("Yes, ship it")),
    ("outlook desktop new", "go with 2\r\n\r\n________________________________\r\nFrom: Bot <{ASK}>\r\nSent: Tuesday, November 10, 2026 10:00 AM\r\nTo: Owner\r\nSubject: Ship the release?\r\n\r\n{QT0}", "go with 2", None),
    ("outlook desktop bold From (no blank)", "2\r\nFrom: Bot <{ASK}>\r\nSent: Tuesday, November 10, 2026 10:00 AM\r\nTo: Owner\r\nSubject: Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook mobile", "2\n\nGet Outlook for iOS<https://aka.ms/o0ukef>\n________________________________\nFrom: Bot <{ASK}>\nSent: Tuesday, November 10, 2026 10:00:00 AM\nTo: owner@example.org <{OWNER}>\nSubject: Ship the release?\n\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook mobile no-sep", "2\n\nGet Outlook for Android\nFrom: Bot <{ASK}>\nSent: Tuesday, November 10, 2026 10:00:00 AM\nTo: owner@example.org\nSubject: Ship the release?\n\n{QT0}", "2", Some("Wait until Monday")),
    ("fastmail", "1\n\nOn Tue, 10 Nov 2026, at 10:00, Bot wrote:\n{QT}", "1", Some("Yes, ship it")),
    ("proton", "2\n\nSent with Proton Mail secure email.\n\nOn Tuesday, November 10th, 2026 at 10:00, Bot <{ASK}> wrote:\n\n{QT}", "2", Some("Wait until Monday")),
    ("hey", "1\n\n--\nOwner\n\nOn November 10, 2026, Bot <{ASK}> wrote:\n{QT}", "1", Some("Yes, ship it")),
    ("superhuman", "Yes, ship it\n\nOn Tue, Nov 10, 2026 at 10:00 AM, Bot <{ASK}> wrote:\n\n{QT}\n", "Yes, ship it", Some("Yes, ship it")),
    ("sig -- block", "2\n\n-- \nOwner Name\nCEO, Example\n+31 6 1234", "2", Some("Wait until Monday")),
    ("sig -- then quote", "2\n-- \nOwner\n\nOn Tue Bot wrote:\n{QT}", "2", Some("Wait until Monday")),
    ("bottom posting", "{QT}\n\n1", "1", Some("Yes, ship it")),
    ("bottom posting w header", "On Tue, Nov 10, 2026 at 10:00 AM Bot <{ASK}> wrote:\n{QT}\n\nWait until Monday please", "Wait until Monday please", None),
    ("bottom posting w signature", "{QT}\n\n2\n\n-- \nOwner", "2", Some("Wait until Monday")),
    ("inline reply", "On Tue, Nov 10, 2026 at 10:00 AM Bot <{ASK}> wrote:\n> Ship the release?\n\nNo, wait, the tests are red.\n\n> 1. Yes, ship it\n", "No, wait, the tests are red.", None),
    ("inline reply no header", "> Ship the release?\n\nWait until Monday\n\n> 1. Yes, ship it\n", "Wait until Monday", Some("Wait until Monday")),
    ("inline reply two parts", "> Ship the release?\n\nNot today.\n\n> 1. Yes, ship it\n\nMonday is fine.", "Not today.\n\nMonday is fine.", None),
    ("dutch", "2\n\nOp di 10 nov 2026 om 10:00 schreef Bot <{ASK}>:\n\n{QT}", "2", Some("Wait until Monday")),
    ("dutch wrapped", "2\n\nOp di 10 nov 2026 om 10:00 schreef Bot <\n{ASK}>:\n\n{QT}", "2", Some("Wait until Monday")),
    ("german", "1\n\nAm Di., 10. Nov. 2026 um 10:00 Uhr schrieb Bot <{ASK}>:\n{QT}", "1", Some("Yes, ship it")),
    ("german apple", "1\n\n> Am 10.11.2026 um 10:00 schrieb Bot <{ASK}>:\n{QT}", "1", Some("Yes, ship it")),
    ("french", "3\n\nLe mar. 10 nov. 2026 à 10:00, Bot <{ASK}> a écrit :\n{QT}", "3", Some("Cancel")),
    ("french wrapped", "3\n\nLe mar. 10 nov. 2026 à 10:00, Bot <{ASK}> a\nécrit :\n{QT}", "3", Some("Cancel")),
    ("spanish", "1\n\nEl mar, 10 nov 2026 a las 10:00, Bot (<{ASK}>) escribió:\n{QT}", "1", Some("Yes, ship it")),
    ("italian", "2\n\nIl giorno mar 10 nov 2026 alle ore 10:00 Bot <{ASK}> ha scritto:\n{QT}", "2", Some("Wait until Monday")),
    ("portuguese", "2\n\nEm ter., 10 de nov. de 2026 às 10:00, Bot <{ASK}> escreveu:\n{QT}", "2", Some("Wait until Monday")),
    ("swedish", "2\n\nDen tis 10 nov. 2026 kl 10:00 skrev Bot <{ASK}>:\n{QT}", "2", Some("Wait until Monday")),
    ("norwegian", "2\n\ntir. 10. nov. 2026 kl. 10:00 skrev Bot <{ASK}>:\n{QT}", "2", Some("Wait until Monday")),
    ("danish wrapped", "2\n\nDen 10. nov. 2026 kl. 10.00 skrev Bot\n<{ASK}>:\n{QT}", "2", Some("Wait until Monday")),
    ("english wrapped 3 lines", "1\n\nOn Tue, Nov 10, 2026 at 10:00 AM Bot Name\n<{ASK}>\nwrote:\n{QT}", "1", Some("Yes, ship it")),
    ("emoji only", "👍\n\nOn Tue Bot wrote:\n{QT}", "👍", None),
    ("yes vs option", "yes", "yes", Some("Yes, ship it")),
    ("Yes, ship it lowercase", "yes, ship it", "yes, ship it", Some("Yes, ship it")),
    ("1.", "1.", "1.", Some("Yes, ship it")),
    ("1)", "1)", "1)", Some("Yes, ship it")),
    ("option 1", "option 1", "option 1", Some("Yes, ship it")),
    ("#1", "#1", "#1", Some("Yes, ship it")),
    ("the first one", "the first one", "the first one", None),
    ("10 minutes", "10 minutes", "10 minutes", None),
    ("2 days", "2 days from now", "2 days from now", None),
    ("3 with text", "3 because the build failed", "3 because the build failed", None),
    ("\" 2 \"", "  2  ", "2", Some("Wait until Monday")),
    ("2 with blank lines", "\n\n2\n", "2", Some("Wait until Monday")),
    ("1.0", "1.0", "1.0", None),
    ("0x2", "0x2", "0x2", None),
    ("+2", "+2", "+2", None),
    ("1e0", "1e0", "1e0", None),
    ("answer starts with >", ">2 weeks is too long, ship it now", ">2 weeks is too long, ship it now", None),
    ("contains On Monday I wrote:", "Ship it.\nOn Monday I wrote: hold.\nNow go.", "Ship it.\nOn Monday I wrote: hold.\nNow go.", None),
    ("On ... then a line with wrote:", "On second thought, wait.\nYou wrote: ship? No.", "On second thought, wait.\nYou wrote: ship? No.", None),
    ("answer with \"Sent from my\"", "Sent from my laptop because phone died: 2", "Sent from my laptop because phone died: 2", None),
    ("answer with From: line", "From: the team: wait\nTo: you: ok\n", "From: the team: wait\nTo: you: ok", None),
    ("answer with -- inside", "wait\n--\nreason: tests", "wait", Some("Wait until Monday")),
    ("answer then forward line", "Yes\n---------- Forwarded message ---------\nFrom: x", "Yes", Some("Yes, ship it")),
    ("empty", "", "", None),
    ("only white space and a quote", "\n\nOn Tue Bot wrote:\n{QT}", "", None),
    ("forwarded instead of reply", "---------- Forwarded message ---------\nFrom: Bot <{ASK}>\nDate: Tue\nSubject: Ship\nTo: <{OWNER}>\n\n{QT0}", "", None),
    ("forward with note", "FYI 1\n\n---------- Forwarded message ---------\nFrom: Bot <{ASK}>\nDate: Tue\nSubject: Ship\nTo: <{OWNER}>\n\n{QT0}", "FYI 1", None),
    ("lowercase with !", "wait until monday!", "wait until monday!", Some("Wait until Monday")),
    ("Cancel?", "Cancel?", "Cancel?", None),
    ("cancel. with sign-off", "Cancel.\n\nThanks,\nOwner", "Cancel.\n\nThanks,\nOwner", Some("Cancel")),
    ("Thanks then 2", "Thanks!\n2", "Thanks!\n2", Some("Wait until Monday")),
    ("Hi, 2, name", "Hi,\n\n2\n\nOwner", "Hi,\n\n2\n\nOwner", Some("Wait until Monday")),
    ("trailing space", "1 \n\nOn Tue Bot wrote:\n{QT}", "1", Some("Yes, ship it")),
    ("zero width", "\u{200b}2", "\u{200b}2", Some("Wait until Monday")),
    ("fullwidth 2", "２", "２", Some("Wait until Monday")),
    ("arabic digit", "٢", "٢", None),
    ("option text plus period", "Yes, ship it.", "Yes, ship it.", Some("Yes, ship it")),
    ("option text with quotes", "\"Cancel\"", "\"Cancel\"", Some("Cancel")),
    ("option text with *bold*", "*Cancel*", "*Cancel*", Some("Cancel")),
    ("outlook html as text", "2\r\n\r\n\r\n\r\nFrom: Bot <{ASK}>\r\nDate: Tuesday, 10 November 2026 at 10:00\r\nTo: Owner <{OWNER}>\r\nSubject: Ship the release?\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook web, Dutch", "2\r\n\r\n________________________________\r\nVan: Bot <{ASK}>\r\nVerzonden: dinsdag 10 november 2026 10:00\r\nAan: Owner\r\nOnderwerp: Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook Dutch, no line", "2\r\n\r\nVan: Bot <{ASK}>\r\nVerzonden: dinsdag 10 november 2026 10:00\r\nAan: Owner\r\nOnderwerp: Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook German", "2\r\n\r\nVon: Bot <{ASK}>\r\nGesendet: Dienstag, 10. November 2026 10:00\r\nAn: Owner\r\nBetreff: Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook French", "2\r\n\r\nDe : Bot <{ASK}>\r\nEnvoyé : mardi 10 novembre 2026 10:00\r\nÀ : Owner\r\nObjet : Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("outlook Spanish", "2\r\n\r\nDe: Bot <{ASK}>\r\nEnviado: martes, 10 de noviembre de 2026 10:00\r\nPara: Owner\r\nAsunto: Ship\r\n\r\n{QT0}", "2", Some("Wait until Monday")),
    ("yahoo", "2\n\n    On Tuesday, November 10, 2026 at 10:00:00 AM GMT+1, Bot <{ASK}> wrote:  \n\n {QT0}", "2", Some("Wait until Monday")),
];

// (name, text part, HTML part, expected answer (None: strip_quoted of the text), expected option)
type HtmlCase = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);
const HTML_CASES: &[HtmlCase] = &[
    ("gmail, real: signature without \"-- \"", "{GMAIL_TEXT}", Some("{GMAIL_HTML}"), Some("1"), Some("It works")),
    ("gmail, real, text part only: the signature stays (nothing marks it)", "{GMAIL_TEXT}", None, None, Some("It works")),
    ("gmail, two lines and a signature", "Not yet.\nThe tests are red.\n\nOwner Name\n\nOn Wed Bot wrote:\n> q\n", Some("<div dir=\"ltr\">Not yet.<div>The tests are red.</div><br clear=\"all\"><div>{SIG}</div></div><br>{QUOTE}"), Some("Not yet.\nThe tests are red."), None),
    ("apple mail, iOS signature and a cited quote", "2\n\nSent from my iPhone\n\n> On 10 Nov 2026, at 10:00, Bot wrote:\n> 1. It works\n", Some("<html><body dir=\"auto\"><div>2</div><div><br><div dir=\"ltr\" id=\"AppleMailSignature\">Owner Name<br>ExampleCo</div><div dir=\"ltr\"><br><blockquote type=\"cite\">On 10 Nov 2026, at 10:00, Bot wrote:<br><br>1. It works</blockquote></div></div></body></html>"), Some("2"), Some("Something is off")),
    ("outlook, signature and reply header", "Something is off\n\nOwner Name\nExampleCo\n\n________________________________\nFrom: Bot\nSent: Tuesday\nTo: Owner\nSubject: q\n\n1. It works", Some("<div style=\"font-family:Aptos\">Something is off</div><div id=\"Signature\"><p>Owner Name</p><p>ExampleCo</p></div><div id=\"appendonsend\"></div><hr><div id=\"divRplyFwdMsg\"><b>From:</b> Bot</div><div>1. It works</div>"), Some("Something is off"), Some("Something is off")),
    ("thunderbird, moz-signature and cite prefix", "1\n\n-- \nOwner\n\nOn 10/11/2026 10:00, Bot wrote:\n> 1. It works", Some("<p>1</p><div class=\"moz-signature\">-- <br>Owner Name<br>ExampleCo</div><div class=\"moz-cite-prefix\">On 10/11/2026 10:00, Bot wrote:<br></div><blockquote type=\"cite\">1. It works</blockquote>"), Some("1"), Some("It works")),
    ("html without marks: the text part decides", "2\n\nOn Tue Bot wrote:\n> 1. It works", Some("<div>2</div><div>On Tue Bot wrote:</div><div>1. It works</div>"), Some("2"), Some("Something is off")),
    ("bottom-posted html: the text part decides", "> 1. It works\n> 2. Something is off\n\n2", Some("<blockquote type=\"cite\">1. It works<br>2. Something is off</blockquote><div>2</div>"), Some("2"), Some("Something is off")),
    ("text only: a signature that the quoted earlier mail of the sender repeats is cut", "Ship it now.\n\nOwner Name\nFounder at ExampleCo\n\nOn Tue, Bot <bot@inbox.example.dev> wrote:\n> Ready?\n>\n>> Should I wait?\n>>\n>> Owner Name\n>> Founder at ExampleCo\n", None, Some("Ship it now."), None),
    ("text only: a last paragraph that the quote does not repeat stays", "Ship it now.\n\nOwner Name\nFounder at ExampleCo\n\nOn Tue, Bot wrote:\n> Ready?\n", None, Some("Ship it now.\n\nOwner Name\nFounder at ExampleCo"), None),
    ("text only: one repeated line is not enough", "2\n\nOwner\n\nOn Tue, Bot wrote:\n> Owner\n", None, Some("2\n\nOwner"), Some("Something is off")),
];

#[test]
fn reply_styles_of_real_mail_clients() {
    let opts = s(&["Yes, ship it", "Wait until Monday", "Cancel"]);
    for (name, text, answer, option) in CASES {
        let text = expand(text);
        let got = strip_quoted(&text);
        assert_eq!(got, *answer, "{name}");
        let picked = if got.is_empty() {
            None
        } else {
            match_option(&got, &opts)
        };
        assert_eq!(picked.as_deref(), *option, "{name}");
        // The answer of a text-only reply reads the same option
        let full = answer_text(&text, None);
        let picked = if full.is_empty() {
            None
        } else {
            match_option(&full, &opts)
        };
        assert_eq!(picked.as_deref(), *option, "{name} (answer_text)");
    }
}

#[test]
fn replies_with_an_html_part() {
    let opts = s(&["It works", "Something is off"]);
    for (name, text, html, answer, option) in HTML_CASES {
        let text = expand(text);
        let html = html.map(expand);
        let want = answer
            .map(str::to_string)
            .unwrap_or_else(|| strip_quoted(&text));
        let got = answer_text(&text, html.as_deref());
        assert_eq!(got, want, "{name}");
        let picked = if got.is_empty() {
            None
        } else {
            match_option(&got, &opts)
        };
        assert_eq!(picked.as_deref(), *option, "{name}");
    }
}

#[test]
fn a_hostile_html_part_is_read_in_linear_time_and_gives_nothing() {
    let t0 = std::time::Instant::now();
    assert_eq!(
        html_answer(Some(&format!("<div class=\"{}", "a".repeat(2_000_000)))),
        None
    );
    assert_eq!(
        html_answer(Some(&format!("<blockquote {}>", " type".repeat(400_000)))),
        None
    );
    assert_eq!(html_answer(Some(&"<".repeat(1_000_000))), None);
    // Debug builds are slower than the reference's 2 s budget; the point is no blow-up
    assert!(t0.elapsed().as_secs() < 10, "{:?}", t0.elapsed());
}
