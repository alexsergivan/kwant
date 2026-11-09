//! The reply guard for pictures (0.8.0 P4): a bot that writes "Here is the mockup…" or "I've
//! shared the screenshot" without putting an image into the chat left the person asking four
//! times (2026-11-09). When a reply claims a picture and its turn shared none, kwnt shares the
//! turn's last screenshot itself, or adds [`NO_PICTURE_LINE`] when there is none.
//!
//! The match is narrow on purpose: a sentence must claim, in the first person or as "here is",
//! that a screenshot, image, picture or mockup is shown, shared or attached. A reply that only
//! mentions the word ("I can take a screenshot", "compress the hero image", "the image you
//! sent") doesn't count, and neither does a question, a negation or a promise.

/// Added to a reply that claims a picture when the turn has none to share.
pub const NO_PICTURE_LINE: &str = "No picture was attached to this reply.";

/// Words naming a picture.
const NOUNS: &[&str] = &[
    "screenshot", "screenshots", "screengrab", "screengrabs", "image", "images", "picture", "pictures", "pic", "pics",
    "photo", "photos", "mockup", "mockups", "mock-up", "mock-ups",
];

/// What a picture was done with, said as done.
const SHARED: &[&str] = &["shared", "attached", "posted", "uploaded", "added", "sent", "dropped", "put", "included"];

/// Who did it, right before one of [`SHARED`]: the bot, never the person or another bot.
const SUBJECTS: &[&str] = &["i", "i've", "we", "we've", "have", "just", "also"];

/// Someone else's picture: "the image you sent".
const OTHERS: &[&str] = &["you", "you've", "your", "they", "they've", "he", "she", "someone", "person", "user"];

/// A word after the picture word that keeps it a picture: not "image optimisation", "image
/// sizes" or "screenshot tool".
const AFTER_NOUN: &[&str] = &[
    "of", "at", "from", "for", "with", "in", "on", "showing", "shows", "show", "taken", "is", "are", "and", "below", "above",
    "here", "i", "that", "which", "so", "to", "now", "again", "too", "this", "as", "into", "just", "was", "were", "has",
    "after", "before", "where", "you", "they", "it",
];

/// Words that turn a claim into a question, a negation, a condition or a promise.
const GUARDS: &[&str] = &[
    "not", "no", "never", "cannot", "unable", "without", "failed", "will", "would", "can", "could", "should", "may", "might",
    "shall", "want", "wants", "if", "once", "when", "let", "ready", "able", "try", "trying", "going", "plan", "planning",
];

fn is_guard(w: &str) -> bool {
    GUARDS.contains(&w) || w.ends_with("n't") || w.ends_with("'ll") || w.ends_with("'d")
}

/// One word, with whether punctuation followed it before the next word.
#[derive(Debug)]
struct Word {
    text: String,
    stop_after: bool,
}

/// Lowercased words of one sentence; curly apostrophes become straight ones.
fn words(sentence: &str) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut cur = String::new();
    for c in sentence.chars() {
        let c = if c == '’' || c == '‘' { '\'' } else { c };
        if c.is_alphanumeric() || c == '\'' || c == '-' {
            cur.extend(c.to_lowercase());
            continue;
        }
        if !cur.is_empty() {
            out.push(Word { text: std::mem::take(&mut cur).trim_matches(|c| c == '\'' || c == '-').to_string(), stop_after: false });
        }
        if !c.is_whitespace() {
            if let Some(last) = out.last_mut() {
                last.stop_after = true;
            }
        }
    }
    if !cur.is_empty() {
        out.push(Word { text: cur.trim_matches(|c| c == '\'' || c == '-').to_string(), stop_after: true });
    } else if let Some(last) = out.last_mut() {
        last.stop_after = true;
    }
    out.retain(|w| !w.text.is_empty());
    out
}

/// The reply's sentences, without code: a fenced block or inline code is never a claim.
fn sentences(text: &str) -> Vec<String> {
    let mut prose = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            prose.push('\n');
            continue;
        }
        if fenced {
            continue;
        }
        let mut inline = false;
        for c in line.chars() {
            if c == '`' {
                inline = !inline;
            } else if !inline {
                prose.push(c);
            }
        }
        prose.push('\n');
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = prose.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        let ends = match c {
            '\n' | '!' | '?' => true,
            // A full stop ends a sentence; one inside "1.5" or "styles.css" doesn't
            '.' => chars.peek().is_none_or(|n| n.is_whitespace()),
            _ => false,
        };
        if ends {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Whether one sentence claims a picture is shown, shared or attached.
fn sentence_claims(sentence: &str) -> bool {
    if sentence.ends_with('?') {
        return false;
    }
    let w = words(sentence);
    for (i, noun) in w.iter().enumerate() {
        if !NOUNS.contains(&noun.text.as_str()) {
            continue;
        }
        // "image optimisation": the picture word only names something else
        let next = w.get(i + 1);
        if !noun.stop_after && next.is_some_and(|n| !AFTER_NOUN.contains(&n.text.as_str())) {
            continue;
        }
        // "the image you sent", "the screenshot they shared": someone else's picture
        if w[i + 1..].iter().take(3).any(|x| OTHERS.contains(&x.text.as_str())) {
            continue;
        }
        let before = &w[..i];
        if before.iter().any(|x| is_guard(&x.text)) {
            continue;
        }
        let near = &before[before.len().saturating_sub(8)..];
        // "Here is the mockup at phone width", "here's a fresh screenshot"
        let here = near.iter().enumerate().any(|(j, x)| {
            x.text == "here's" || (x.text == "here" && near.get(j + 1).is_some_and(|n| n.text == "is" || n.text == "are"))
        });
        if here {
            return true;
        }
        // "I've shared the screenshot", "Attached a picture of…", "see the attached mockup"
        let shared_before = near.iter().enumerate().any(|(j, x)| {
            if !SHARED.contains(&x.text.as_str()) {
                return false;
            }
            let at = before.len() - near.len() + j;
            at == 0
                || before[..at].iter().rev().take(2).any(|s| SUBJECTS.contains(&s.text.as_str()))
                || before[..at].iter().any(|s| s.text == "see" || s.text == "look")
        });
        if shared_before {
            return true;
        }
        // "the screenshot is attached", "took a screenshot and shared it in this chat", "the mockup below:"
        let after = &w[i + 1..];
        if !noun.stop_after
            && after.first().is_some_and(|a| a.text == "below" || a.text == "above")
            && (after[0].stop_after || after.get(1).is_some_and(|n| n.text == "shows" || n.text == "show"))
        {
            return true;
        }
        if after.first().is_some_and(|a| a.text == "is" || a.text == "are")
            && after.get(1).is_some_and(|b| b.text == "below" || b.text == "above" || SHARED.contains(&b.text.as_str()))
        {
            return true;
        }
        for (j, x) in after.iter().enumerate().take(8) {
            if is_guard(&x.text) || OTHERS.contains(&x.text.as_str()) {
                break;
            }
            if SHARED.contains(&x.text.as_str()) {
                let then = after.get(j + 1).map(|n| n.text.as_str());
                if matches!(then, Some("it" | "them" | "here" | "in" | "into" | "to" | "above" | "below")) || x.stop_after {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a bot's reply says it shows, shares or attaches a picture. A reply that embeds one
/// itself (a markdown image) already shows it.
pub fn claims_picture(reply: &str) -> bool {
    if reply.contains("![") || reply.contains(NO_PICTURE_LINE) {
        return false;
    }
    sentences(reply).iter().any(|s| sentence_claims(s))
}

/// The reply with [`NO_PICTURE_LINE`] on its own line at the end.
pub fn with_no_picture_line(reply: &str) -> String {
    let body = reply.trim_end();
    if body.is_empty() {
        NO_PICTURE_LINE.to_string()
    } else {
        format!("{body}\n\n{NO_PICTURE_LINE}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The replies from Oleks's chat (conversation-2026-11-09.md) that claimed a picture.
    #[test]
    fn claims_from_the_chat_that_had_no_picture() {
        for reply in [
            "Here is the mockup at phone width (390 px). The bar sits under the hero.",
            "Here is the mockup at phone width (390×844 px), taken from the browser panel…",
            "I took a fresh screenshot… shared it in this chat",
            "I took a fresh screenshot and shared it in this chat.",
        ] {
            assert!(claims_picture(reply), "{reply}");
        }
    }

    #[test]
    fn other_claims() {
        for reply in [
            "Here's a fresh screenshot of the page.",
            "Here are the screenshots: light and dark.",
            "I've shared the screenshot.",
            "I’ve attached a picture of the new footer.",
            "I just posted the mockup.",
            "Attached a screenshot of the phone layout.",
            "We've added the images of both themes",
            "See the attached mockup for the spacing.",
            "The screenshot is attached.",
            "Screenshots are below.",
            "The mockup below shows the orange button.",
            "Mockup above: the bar at 390 px.",
            "Done!\n\nHere is the screenshot I would use for the post",
            "Restyled from web/static/css/styles.css. Here is the image with the brand orange #d96716.",
        ] {
            assert!(claims_picture(reply), "{reply}");
        }
    }

    #[test]
    fn mentions_that_are_not_claims() {
        for reply in [
            "I can take a screenshot if you want.",
            "Want me to share a screenshot?",
            "Shall I post the mockup here?",
            "The mockup uses the site's own CSS.",
            "Compress the hero image and add alt text to every image tag.",
            "Image optimisation would save 300 KB.",
            "The image above the fold loads late.",
            "I couldn't take a screenshot: the browser didn't answer.",
            "I haven't shared the screenshot yet.",
            "No screenshot was taken.",
            "I'll share the mockup once Growth finishes it.",
            "Thanks, I looked at the image you sent: the button is red.",
            "The screenshot you shared shows the old footer.",
            "If the page looks right, I can also push the branch and open a PR.",
            "The `screenshot` helper is shared by both tests.",
            "```\n// Here is the screenshot\n```",
            "Run browser_screenshot with width 390 to see it.",
            "I'll take the screenshot when the deploy finishes.",
            "Here is the plan: fix the footer, then the header.",
            "Here is the mockup: ![mockup](/api/attachments/x/content)",
        ] {
            assert!(!claims_picture(reply), "{reply}");
        }
    }

    #[test]
    fn our_own_line_never_claims() {
        assert!(!claims_picture(NO_PICTURE_LINE));
        assert!(!claims_picture(&with_no_picture_line("Here is the mockup.")));
    }

    #[test]
    fn the_line_goes_at_the_end() {
        assert_eq!(with_no_picture_line("Here is the mockup.\n"), "Here is the mockup.\n\nNo picture was attached to this reply.");
        assert_eq!(with_no_picture_line("  "), NO_PICTURE_LINE);
    }
}
