//! The rules that turn a conversation into a loose end, or into nothing. Plain string rules, so
//! every case can say exactly which sentence made it one; the quote is always cut from the
//! message itself, so it is verbatim by construction.

use super::{clip, Candidate, CaseKind, ConvKind, Conversation, Evidence, Msg, MAX_QUOTE_CHARS};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};

/// How old a conversation's last message must be before it counts.
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    pub now: DateTime<Utc>,
    /// Email and GitHub.
    pub stale_days: i64,
    /// kwant's own threads, tasks and questions.
    pub kwant_stale_days: i64,
}

impl Rules {
    fn stale_for(&self, c: &Conversation) -> i64 {
        match c.kind {
            ConvKind::KwantThread | ConvKind::KwantQuestion | ConvKind::KwantDelegation => self.kwant_stale_days,
            _ => self.stale_days,
        }
    }
}

/// Someone saying they will come back to it.
const THEIR_PROMISES: &[&str] = &[
    "get back to you",
    "get back to me",
    "will be in touch",
    "we will respond",
    "we'll respond",
    "will respond within",
    "will reply",
    "will contact you",
    "will follow up",
    "will update you",
    "will let you know",
    "'ll let you know",
    "we are reviewing",
    "we're reviewing",
    "is under review",
    "is being reviewed",
    "will be processed",
    "will send you",
    "'ll send you",
    "will be sent to you",
    "will reach out",
    "'ll reach out",
    "will look into",
    "'ll look into",
    "when i return",
    "when i'm back",
];

/// The person saying they will do something.
const MY_PROMISES: &[&str] = &[
    "i'll send",
    "i will send",
    "i'll get back",
    "i will get back",
    "i'll follow up",
    "i will follow up",
    "i'll check",
    "i will check",
    "i'll look into",
    "i will look into",
    "i'll share",
    "i will share",
    "i'll update you",
    "i will update you",
    "i'll confirm",
    "i will confirm",
    "i'll let you know",
    "i will let you know",
    "i'll have it",
    "i'll have the",
    "i will have the",
    "i'll do it",
    "i'll prepare",
    "i will prepare",
    "i'll pay",
    "i will pay",
    "i'll call",
    "i will call",
    "i promise",
    "let me check",
];

/// A message that expects an answer.
const REQUESTS: &[&str] = &[
    "?",
    "please",
    "let me know",
    "could you",
    "can you",
    "would you",
    "any update",
    "following up",
    "follow up on",
    "waiting for",
    "waiting on",
    "look forward to hearing",
    "looking forward to hearing",
    "get back to me",
    "kindly",
    "need your",
    "need you to",
    "awaiting your",
    "awaiting a reply",
    "awaiting an answer",
];

/// Sentences that look like requests but expect nothing: sign-offs and rhetorical questions.
const PLEASANTRIES: &[&str] = &[
    "let me know if you have any questions",
    "let me know if you have questions",
    "let me know if you need anything",
    "let me know if there's anything",
    "can you believe",
    "how cool is that",
    "isn't it",
    "wasn't it",
    "how are you",
];

/// Whether some sentence of `text` asks for something.
fn asks(text: &str) -> bool {
    sentences(text).iter().any(|s| has_any(s, REQUESTS) && !has_any(s, PLEASANTRIES))
}

/// The first sentence of `text` that asks for something, or its first sentence.
fn request_quote(text: &str) -> &str {
    let all = sentences(text);
    all.iter().find(|s| has_any(s, REQUESTS) && !has_any(s, PLEASANTRIES)).copied().unwrap_or_else(|| quote_for(text, REQUESTS))
}

fn has_any(text: &str, phrases: &[&str]) -> bool {
    let t = normalize_quotes(&text.to_lowercase());
    phrases.iter().any(|p| t.contains(p))
}

/// Curly apostrophes as straight ones, so "I’ll" matches "i'll".
fn normalize_quotes(s: &str) -> String {
    s.replace(['\u{2019}', '\u{2018}'], "'")
}

const ABBREVIATIONS: &[&str] = &["no", "nr", "mr", "mrs", "ms", "dr", "st", "vs", "etc", "e.g", "i.e", "approx", "ref", "inv"];

/// The sentences of `text`, each a slice of it (so a quote cut from one is verbatim).
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        // "No. 4471", "Mr. Smith", "e.g. this": not the end of a sentence
        if ch == '.' {
            let word = text[start..i].rsplit(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_lowercase();
            let next = text[i + 1..].chars().next();
            if ABBREVIATIONS.contains(&word.as_str()) || next.is_some_and(|n| !n.is_whitespace()) {
                continue;
            }
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let end = i + ch.len_utf8();
            let s = text[start..end].trim();
            if !s.is_empty() {
                out.push(s);
            }
            start = end;
        }
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// The first sentence of `text` holding one of `phrases`, or its first sentence.
fn quote_for<'a>(text: &'a str, phrases: &[&str]) -> &'a str {
    let all = sentences(text);
    all.iter()
        .find(|s| has_any(s, phrases))
        .or_else(|| all.iter().find(|s| s.chars().any(char::is_alphanumeric)))
        .copied()
        .unwrap_or("")
}

fn evidence(m: &Msg, quote: &str) -> Option<Evidence> {
    let quote = clip(quote.trim(), MAX_QUOTE_CHARS);
    (!quote.is_empty()).then(|| Evidence { quote, from: m.from.clone(), at: m.at, outbound: m.outbound })
}

/// "Thanks!", "Perfect, see you then." — a message that closes an exchange.
fn is_closer(text: &str) -> bool {
    let t = text.trim();
    t.chars().count() <= 120 && !asks(t) && !has_any(t, MY_PROMISES)
}

fn days(n: i64) -> String {
    if n == 1 {
        "1 day".into()
    } else {
        format!("{n} days")
    }
}

/// The loose end in `c`, if there is one.
pub fn detect(c: &Conversation, rules: &Rules) -> Option<Candidate> {
    if !c.open {
        return None;
    }
    let stale = Duration::days(rules.stale_for(c));
    let human: Vec<&Msg> = c.messages.iter().filter(|m| !m.auto).collect();
    let last = *human.last()?;
    if rules.now - last.at < stale {
        return None;
    }
    // Only lists, newsletters and bots on the other side, and the person never wrote: not theirs
    if !c.messages.iter().any(|m| m.outbound) && c.messages.iter().all(|m| m.bulk) {
        return None;
    }
    let age = (rules.now - last.at).num_days();
    let who = c.counterpart_name.clone().or_else(|| c.counterpart.clone()).unwrap_or_else(|| "them".into());
    let made = |kind: CaseKind, why: String, ev: Vec<Evidence>, deadline: Option<NaiveDate>| -> Option<Candidate> {
        if ev.is_empty() {
            return None;
        }
        Some(Candidate {
            kind,
            dedupe_key: c.dedupe_key(),
            source: c.source,
            source_id: c.source_id.clone(),
            source_ref: c.key.clone(),
            conv_kind: c.kind,
            // "Re: Fwd: Claim 4471" shows as "Claim 4471"
            title: match c.subject.trim() {
                "" => "(no subject)".into(),
                s if c.kind == ConvKind::Email => clip(super::draft::reply_subject(s).trim_start_matches("Re: "), 200),
                s => clip(s, 200),
            },
            counterpart: c.counterpart.clone(),
            counterpart_name: c.counterpart_name.clone(),
            reference: c.reference.clone().or_else(|| find_reference(c)),
            link: c.link.clone(),
            why,
            evidence: ev,
            last_activity_at: last.at,
            deadline,
            reply_to_message_id: c.messages.iter().rev().find_map(|m| m.message_id.clone()),
            space_id: c.space_id.clone(),
            kwant_id: c.kwant_id.clone(),
        })
    };

    match c.kind {
        ConvKind::KwantDelegation => {
            if !c.failed {
                return None;
            }
            let m = c.messages.iter().rev().find(|m| !m.outbound).unwrap_or(last);
            let ev = evidence(m, quote_for(&m.text, &["fail", "error", "interrupt", "stopped"]))?;
            return made(
                CaseKind::FailedTask,
                format!("{who} could not finish this task {} ago and nobody retried it.", days(age)),
                vec![ev],
                None,
            );
        }
        ConvKind::KwantQuestion => {
            let ev = evidence(last, quote_for(&last.text, &["?"]))?;
            return made(
                CaseKind::UnansweredQuestion,
                format!("{who} asked you this {} ago and is still waiting for your answer.", days(age)),
                vec![ev],
                None,
            );
        }
        _ => {}
    }

    if last.outbound {
        // Written to an address nobody reads: no answer was ever coming
        if c.kind == ConvKind::Email && c.counterpart.as_deref().is_some_and(super::imap::is_noreply) {
            return None;
        }
        // An auto-reply after the person's message that promised an answer: that promise lapsed
        // (`last` is the newest human message; an acknowledgement stamped the same second counts)
        let ack = c.messages.iter().rev().find(|m| m.auto && m.at >= last.at && !m.outbound && has_any(&m.text, THEIR_PROMISES));
        // ... to a message that asked for something: an out-of-office after "Thanks, all done" promises nothing
        if let Some(auto) = ack.filter(|_| asks(&last.text)) {
            let mut ev: Vec<Evidence> = evidence(last, request_quote(&last.text)).into_iter().collect();
            ev.extend(evidence(auto, quote_for(&auto.text, THEIR_PROMISES)));
            return made(
                CaseKind::PromiseLapsed,
                format!("You wrote to {who} {} ago. They promised an answer and none came.", days(age)),
                ev,
                None,
            );
        }
        if has_any(&last.text, MY_PROMISES) {
            let q = quote_for(&last.text, MY_PROMISES);
            let deadline = deadline_in(q, last.at).filter(|d| *d < rules.now.date_naive());
            let why = match deadline {
                Some(d) => format!("You promised {who} something by {} and nothing followed for {}.", d.format("%-d %b"), days(age)),
                None => format!("You promised {who} something {} ago and nothing followed.", days(age)),
            };
            return made(CaseKind::YouPromised, why, evidence(last, q).into_iter().collect(), deadline);
        }
        let expects_answer = match c.kind {
            // A message to a bot that never answered
            ConvKind::KwantThread => true,
            _ => asks(&last.text) && !is_closer(&last.text),
        };
        if expects_answer {
            return made(
                CaseKind::AwaitingReply,
                format!("You wrote to {who} {} ago and got no reply.", days(age)),
                evidence(last, request_quote(&last.text)).into_iter().collect(),
                None,
            );
        }
        if matches!(c.kind, ConvKind::Issue | ConvKind::PullRequest) && c.mine {
            return stale_issue(c, last, age, made);
        }
        return None;
    }

    // The other side wrote last. A bot's closing "Shall I go ahead?" is not tracked here: its
    // real questions to the person are `ask_user` questions, which are their own source
    if last.bulk || c.kind == ConvKind::KwantThread {
        return None;
    }
    if has_any(&last.text, THEIR_PROMISES) {
        let q = quote_for(&last.text, THEIR_PROMISES);
        let deadline = deadline_in(q, last.at).filter(|d| *d < rules.now.date_naive());
        let why = match deadline {
            Some(d) => format!("{who} promised to get back to you by {}; it has been {} and nothing came.", d.format("%-d %b"), days(age)),
            None => format!("{who} said they would get back to you {} ago and never did.", days(age)),
        };
        return made(CaseKind::PromiseLapsed, why, evidence(last, q).into_iter().collect(), deadline);
    }
    // Asked in an exchange the person is part of: they owe the answer. A cold message is not theirs.
    let in_exchange = c.messages.iter().any(|m| m.outbound) || c.mine;
    if in_exchange && asks(&last.text) {
        return made(
            CaseKind::YouOweReply,
            format!("{who} wrote to you {} ago and is waiting for your answer.", days(age)),
            evidence(last, request_quote(&last.text)).into_iter().collect(),
            None,
        );
    }
    if matches!(c.kind, ConvKind::Issue | ConvKind::PullRequest) && c.mine {
        return stale_issue(c, last, age, made);
    }
    None
}

fn stale_issue(
    c: &Conversation,
    last: &Msg,
    age: i64,
    made: impl Fn(CaseKind, String, Vec<Evidence>, Option<NaiveDate>) -> Option<Candidate>,
) -> Option<Candidate> {
    let what = if c.kind == ConvKind::PullRequest { "Your pull request" } else { "Your issue" };
    // The title is quotable evidence too: it is part of the source text
    let first = c.messages.first().unwrap_or(last);
    let ev = evidence(first, quote_for(&first.text, &[])).or_else(|| {
        Some(Evidence { quote: clip(c.subject.trim(), MAX_QUOTE_CHARS), from: first.from.clone(), at: first.at, outbound: first.outbound })
            .filter(|e| !e.quote.is_empty())
    })?;
    made(CaseKind::StaleIssue, format!("{what} has had no activity for {}.", days(age)), vec![ev], None)
}

/// Words a reference number follows: "claim no. 4471", "Ticket #A-19", "Ref: XK-22".
const REFERENCE_WORDS: &[&str] = &[
    "claim", "case", "ticket", "ref", "reference", "order", "invoice", "policy", "booking", "application",
    "request", "account", "confirmation", "tracking", "incident", "inv", "po", "reservation", "contract", "file",
];
const FILLERS: &[&str] = &["no", "no.", "nr", "nr.", "number", "num", "#", "id", "is", ":", "-", "n°", "№"];

/// The reference number a follow-up should quote, from the subject first, then the messages.
pub fn find_reference(c: &Conversation) -> Option<String> {
    reference_in(&c.subject).or_else(|| c.messages.iter().find_map(|m| reference_in(&m.text)))
}

/// A reference number in `text`: after a word like "claim" or "ticket", or a `#1234`.
pub fn reference_in(text: &str) -> Option<String> {
    let trim = |w: &str| w.trim_matches(|ch: char| matches!(ch, ',' | '.' | ';' | ':' | '?' | '!' | '(' | ')' | '[' | ']' | '"' | '\'' | '<' | '>')).to_string();
    let words: Vec<&str> = text.split_whitespace().collect();
    let looks_like_ref = |w: &str| {
        let w = w.trim_start_matches('#');
        w.chars().count() >= 3
            && w.chars().count() <= 30
            && w.chars().any(|ch| ch.is_ascii_digit())
            && w.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '/' | '_' | '.'))
    };
    for (i, w) in words.iter().enumerate() {
        let key = trim(w).to_lowercase();
        let key = key.trim_end_matches(['#', ':']);
        if REFERENCE_WORDS.contains(&key) {
            let mut j = i + 1;
            while j < words.len() && j <= i + 3 && FILLERS.contains(&trim(words[j]).to_lowercase().as_str()) {
                j += 1;
            }
            if let Some(next) = words.get(j) {
                let cand = trim(next);
                // "No.4471" glued to its filler
                let lower = cand.to_lowercase();
                let glued = ["no.", "nr.", "n°", "№"].iter().find_map(|p| lower.strip_prefix(p).map(|r| cand.len() - r.len()));
                let cand = glued.and_then(|at| cand.get(at..)).unwrap_or(&cand);
                let cand = cand.trim_start_matches(['#', ':']);
                if looks_like_ref(cand) {
                    return Some(cand.trim_end_matches('.').to_string());
                }
            }
            // "claim #4471" where '#' was its own word was skipped as a filler; "case:4471" glued
            if let Some((_, after)) = w.split_once([':', '#']) {
                let after = trim(after);
                if looks_like_ref(&after) {
                    return Some(after);
                }
            }
        }
    }
    words.iter().map(|w| trim(w)).find(|w| w.starts_with('#') && w.len() >= 4 && w[1..].chars().all(|ch| ch.is_ascii_digit())).map(|w| w.to_string())
}

/// The date a promise names ("by Friday", "by 3 November", "by 2026-11-03", "tomorrow", "by the
/// end of the week"), counted from when it was written.
pub fn deadline_in(sentence: &str, written: DateTime<Utc>) -> Option<NaiveDate> {
    let t = normalize_quotes(&sentence.to_lowercase());
    let base = written.date_naive();
    if t.contains("tomorrow") {
        return base.succ_opt();
    }
    if t.contains("end of the week") || t.contains("end of this week") || t.contains("end of week") {
        return Some(next_weekday(base, Weekday::Fri, true));
    }
    if t.contains("end of the month") || t.contains("end of this month") || t.contains("end of month") {
        let (y, m) = if base.month() == 12 { (base.year() + 1, 1) } else { (base.year(), base.month() + 1) };
        return NaiveDate::from_ymd_opt(y, m, 1).and_then(|d| d.pred_opt());
    }
    let words: Vec<String> = t
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '.' | '!' | '?' | ';' | '(' | ')'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let by = words.iter().position(|w| w == "by" || w == "before" || w == "until")?;
    let rest = &words[by + 1..];
    let rest: Vec<&str> = rest.iter().map(String::as_str).filter(|w| *w != "the" && *w != "next" && *w != "this").collect();
    let first = *rest.first()?;
    if let Ok(d) = NaiveDate::parse_from_str(first, "%Y-%m-%d") {
        return Some(d);
    }
    if let Some(wd) = weekday(first) {
        return Some(next_weekday(base, wd, false));
    }
    let day_of = |w: &str| w.trim_end_matches("st").trim_end_matches("nd").trim_end_matches("rd").trim_end_matches("th").parse::<u32>().ok();
    let (month, day) = match (rest.first().and_then(|w| month(w)), rest.get(1).and_then(|w| day_of(w))) {
        (Some(m), Some(d)) => (m, d),
        _ => match (rest.first().and_then(|w| day_of(w)), rest.get(1).and_then(|w| month(w))) {
            (Some(d), Some(m)) => (m, d),
            _ => return None,
        },
    };
    let this_year = NaiveDate::from_ymd_opt(base.year(), month, day)?;
    // "by 3 January" written in December means next year
    if this_year < base - Duration::days(30) {
        NaiveDate::from_ymd_opt(base.year() + 1, month, day)
    } else {
        Some(this_year)
    }
}

/// The first `wd` after `base` (on `base` itself when `same_day_ok`).
fn next_weekday(base: NaiveDate, wd: Weekday, same_day_ok: bool) -> NaiveDate {
    let mut d = if same_day_ok { base } else { base + Duration::days(1) };
    while d.weekday() != wd {
        d += Duration::days(1);
    }
    d
}

fn weekday(w: &str) -> Option<Weekday> {
    Some(match w {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

fn month(w: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    if w.len() < 3 {
        return None;
    }
    let full = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    MONTHS.iter().position(|m| w.starts_with(m)).filter(|&i| full[i].starts_with(w) || w.len() == 3 || w == "sept").map(|i| i as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zombie::{quote_in, SourceKind};

    fn now() -> DateTime<Utc> {
        "2026-10-20T09:00:00Z".parse().unwrap()
    }
    fn ago(d: i64) -> DateTime<Utc> {
        now() - Duration::days(d)
    }
    fn rules() -> Rules {
        Rules { now: now(), stale_days: 14, kwant_stale_days: 3 }
    }
    fn msg(outbound: bool, days_ago: i64, text: &str) -> Msg {
        Msg {
            from: if outbound { "me@example.com".into() } else { "them@example.org".into() },
            outbound,
            at: ago(days_ago),
            text: text.into(),
            auto: false,
            bulk: false,
            message_id: None,
        }
    }
    fn email(subject: &str, messages: Vec<Msg>) -> Conversation {
        let mut c = Conversation::new(SourceKind::Imap, "src-1", "thread-1", ConvKind::Email, subject);
        c.counterpart = Some("them@example.org".into());
        c.messages = messages;
        c
    }
    fn assert_verbatim(c: &Conversation, cand: &Candidate) {
        for e in &cand.evidence {
            assert!(c.texts().any(|t| quote_in(t, &e.quote)), "quote not in the source: {:?}", e.quote);
        }
    }

    #[test]
    fn you_wrote_last_asking_and_no_reply_for_14_days() {
        let c = email("Insurance claim 4471", vec![msg(false, 50, "Your claim was received."), msg(true, 41, "Hello. Could you tell me the status of claim 4471? Thanks")]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::AwaitingReply);
        assert_eq!(cand.reference.as_deref(), Some("4471"));
        assert_eq!(cand.age_days(now()), 41);
        assert!(cand.why.contains("41 days"), "{}", cand.why);
        assert_eq!(cand.evidence[0].quote, "Could you tell me the status of claim 4471?");
        assert_verbatim(&c, &cand);
    }

    #[test]
    fn too_recent_is_not_a_case() {
        let c = email("Status", vec![msg(true, 13, "Any update on this?")]);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn a_thank_you_that_closes_the_exchange_is_not_a_case() {
        let c = email("Lunch", vec![msg(false, 30, "See you at noon?"), msg(true, 29, "Perfect, thanks!")]);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn a_statement_without_a_request_is_not_a_case() {
        let c = email("Slides", vec![msg(true, 30, "Attached are the slides from today's talk.")]);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn an_auto_reply_is_not_an_answer_and_its_promise_lapses() {
        let mut auto = msg(false, 40, "We received your request and will respond within 5 business days. Ticket #88213.");
        auto.auto = true;
        let c = email("Refund", vec![msg(true, 41, "Please refund order 5512, it arrived broken."), auto]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::PromiseLapsed);
        assert_eq!(cand.evidence.len(), 2);
        assert_eq!(cand.evidence[1].quote, "We received your request and will respond within 5 business days.");
        assert_eq!(cand.reference.as_deref(), Some("5512"));
        assert_eq!(cand.last_activity_at, ago(41), "the age counts from the person's message");
        assert_verbatim(&c, &cand);
    }

    #[test]
    fn an_acknowledgement_stamped_the_same_second_still_counts() {
        let mut auto = msg(false, 40, "We received your request and will respond within 3 business days.");
        auto.auto = true;
        // Sorted by time, the acknowledgement can land before the person's own message
        let c = email("Visa letter", vec![auto, msg(true, 40, "Please issue the support letter, reference VX-2210.")]);
        assert_eq!(detect(&c, &rules()).unwrap().kind, CaseKind::PromiseLapsed);
    }

    #[test]
    fn sign_offs_and_rhetorical_questions_ask_for_nothing() {
        let c = email("Slides", vec![msg(true, 30, "Here are the slides from today. Let me know if you have questions.")]);
        assert!(detect(&c, &rules()).is_none());
        let c = email("Concert", vec![msg(true, 31, "The concert was amazing."), msg(false, 30, "Haha, can you believe how loud it was?")]);
        assert!(detect(&c, &rules()).is_none());
        // A real question next to a sign-off still counts, and is the one quoted
        let c = email("Slides", vec![msg(true, 30, "Could you share the recording? Let me know if you have questions.")]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.evidence[0].quote, "Could you share the recording?");
    }

    #[test]
    fn awaiting_a_countersignature_expects_an_answer() {
        let c = email("Lease", vec![msg(true, 30, "Sending over the signed lease; awaiting your countersignature.")]);
        assert_eq!(detect(&c, &rules()).unwrap().kind, CaseKind::AwaitingReply);
    }

    #[test]
    fn a_plain_out_of_office_still_leaves_the_question_open() {
        let mut ooo = msg(false, 20, "I am out of the office.");
        ooo.auto = true;
        let c = email("Contract", vec![msg(true, 21, "Can you send the signed contract?"), ooo]);
        assert_eq!(detect(&c, &rules()).unwrap().kind, CaseKind::AwaitingReply);
    }

    #[test]
    fn their_promise_that_lapsed() {
        let c = email("Quote", vec![msg(true, 30, "What would it cost?"), msg(false, 25, "Thanks! I’ll get back to you by Friday with a quote.")]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::PromiseLapsed);
        assert!(cand.deadline.is_some());
        assert!(cand.why.contains("by "), "{}", cand.why);
        assert_verbatim(&c, &cand);
    }

    #[test]
    fn my_own_promise_that_nobody_followed_up() {
        let c = email("Invoice", vec![msg(false, 20, "Can you send the invoice?"), msg(true, 19, "Sure, I'll send it by Monday.")]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::YouPromised);
        assert_eq!(cand.evidence[0].quote, "Sure, I'll send it by Monday.");
    }

    #[test]
    fn they_asked_in_an_exchange_and_you_never_answered() {
        let c = email("Apartment", vec![msg(true, 40, "Is the flat still available?"), msg(false, 35, "Yes. When would you like to visit?")]);
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::YouOweReply);
        assert_eq!(cand.evidence[0].quote, "When would you like to visit?");
    }

    #[test]
    fn a_cold_inbound_question_is_not_theirs() {
        let c = email("Partnership", vec![msg(false, 40, "Would you like to buy backlinks?")]);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn newsletters_never_count() {
        let mut n = msg(false, 40, "Please confirm your subscription? We will get back to you.");
        n.bulk = true;
        assert!(detect(&email("Weekly digest", vec![n.clone()]), &rules()).is_none());
        // Even after the person wrote, a bulk last message is not a promise or a question
        assert!(detect(&email("Digest", vec![msg(true, 50, "Thanks"), n]), &rules()).is_none());
    }

    #[test]
    fn a_resolved_thread_is_not_a_case() {
        let c = email("Booking", vec![msg(true, 40, "Can you confirm booking AB-1234?"), msg(false, 39, "Confirmed, see you on the 3rd.")]);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn issues_closed_are_never_cases_and_open_stale_ones_of_mine_are() {
        let mut c = Conversation::new(SourceKind::Github, "gh", "https://github.com/o/r/issues/7", ConvKind::Issue, "Crash on start");
        c.mine = true;
        c.reference = Some("o/r#7".into());
        c.messages = vec![msg(true, 60, "It crashes when the config is empty.")];
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::StaleIssue);
        assert_eq!(cand.reference.as_deref(), Some("o/r#7"));
        c.open = false;
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn a_pull_request_of_someone_else_that_went_quiet_is_not_mine() {
        let mut c = Conversation::new(SourceKind::Github, "gh", "k", ConvKind::PullRequest, "Bump deps");
        c.messages = vec![msg(false, 60, "Bumps serde.")];
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn kwant_rules_use_their_own_shorter_threshold() {
        let mut c = Conversation::new(SourceKind::Kwant, "kwant", "t1", ConvKind::KwantThread, "Research");
        c.counterpart_name = Some("Scout".into());
        c.messages = vec![msg(true, 4, "Find me three hotels in Lisbon")];
        let cand = detect(&c, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::AwaitingReply);
        assert!(cand.why.starts_with("You wrote to Scout 4 days ago"), "{}", cand.why);
        c.messages[0].at = ago(2);
        assert!(detect(&c, &rules()).is_none());
    }

    #[test]
    fn a_failed_delegation_and_an_open_question() {
        let mut d = Conversation::new(SourceKind::Kwant, "kwant", "d1", ConvKind::KwantDelegation, "Write the report");
        d.failed = true;
        d.messages = vec![msg(true, 6, "Write the report"), msg(false, 5, "The task failed: the engine timed out.")];
        assert_eq!(detect(&d, &rules()).unwrap().kind, CaseKind::FailedTask);
        d.failed = false;
        assert!(detect(&d, &rules()).is_none());

        let mut q = Conversation::new(SourceKind::Kwant, "kwant", "q1", ConvKind::KwantQuestion, "Which budget?");
        q.messages = vec![msg(false, 5, "Which budget should I use for the campaign?")];
        let cand = detect(&q, &rules()).unwrap();
        assert_eq!(cand.kind, CaseKind::UnansweredQuestion);
        assert_eq!(cand.evidence[0].quote, "Which budget should I use for the campaign?");
    }

    #[test]
    fn references() {
        assert_eq!(reference_in("Re: Claim No. 4471-B status").as_deref(), Some("4471-B"));
        assert_eq!(reference_in("your ticket #88213 is open").as_deref(), Some("88213"));
        assert_eq!(reference_in("Ref: XK-22").as_deref(), Some("XK-22"));
        assert_eq!(reference_in("Order number: 5512.").as_deref(), Some("5512"));
        assert_eq!(reference_in("see #1234 for details").as_deref(), Some("#1234"));
        assert_eq!(reference_in("the status of claim No. 4471-B?").as_deref(), Some("4471-B"));
        assert_eq!(reference_in("Policy No.77120 renewal").as_deref(), Some("77120"));
        assert_eq!(reference_in("the case is closed"), None);
        assert_eq!(reference_in("order of 3 items"), None);
    }

    #[test]
    fn deadlines() {
        let wed: DateTime<Utc> = "2026-10-07T10:00:00Z".parse().unwrap();
        assert_eq!(deadline_in("I'll send it by Friday.", wed), NaiveDate::from_ymd_opt(2026, 10, 9));
        assert_eq!(deadline_in("by Wednesday", wed), NaiveDate::from_ymd_opt(2026, 10, 14));
        assert_eq!(deadline_in("we will reply by 3 November", wed), NaiveDate::from_ymd_opt(2026, 11, 3));
        assert_eq!(deadline_in("by November 3rd", wed), NaiveDate::from_ymd_opt(2026, 11, 3));
        assert_eq!(deadline_in("by 2026-10-30", wed), NaiveDate::from_ymd_opt(2026, 10, 30));
        assert_eq!(deadline_in("tomorrow", wed), NaiveDate::from_ymd_opt(2026, 10, 8));
        assert_eq!(deadline_in("by the end of the week", wed), NaiveDate::from_ymd_opt(2026, 10, 9));
        assert_eq!(deadline_in("soon", wed), None);
        let dec: DateTime<Utc> = "2026-12-20T10:00:00Z".parse().unwrap();
        assert_eq!(deadline_in("by January 5", dec), NaiveDate::from_ymd_opt(2027, 1, 5));
    }
}
