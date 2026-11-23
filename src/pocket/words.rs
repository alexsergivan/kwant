//! Guest names: 1-20 characters, trimmed, no links or control characters, and a small word
//! filter (plan section 8 rule 5). The filter is deliberately small and English-first: it stops
//! the obvious names on a big screen, the host removes the rest with one tap.

use unicode_normalization::UnicodeNormalization;

pub const NAME_MAX: usize = 20;

/// Why a name was refused, as the guest page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    NotAllowed,
}

impl NameError {
    pub fn code(&self) -> &'static str {
        match self {
            NameError::Empty => "name_empty",
            NameError::TooLong => "name_too_long",
            NameError::NotAllowed => "name_not_allowed",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            NameError::Empty => "Type a name.",
            NameError::TooLong => "Names are at most 20 characters.",
            NameError::NotAllowed => "Pick another name.",
        }
    }
}

/// Words refused anywhere in a name, even inside another word.
const ANYWHERE: &[&str] = &["fuck", "shit", "nigg", "fagg", "retard", "whore", "hitler", "rapist", "wanker", "bastard", "asshole"];
/// Words refused only as a whole word (they hide inside ordinary names otherwise: Dickens, Scunthorpe).
const WHOLE: &[&str] = &[
    "fag", "cunt", "cunts", "dick", "cock", "rape", "nazi", "twat", "slut", "spic", "kike", "chink", "pussy", "wank", "bitch", "porn", "sex", "penis",
    "anus", "tits", "boobs", "dildo", "nazis", "kkk",
];

/// The name as shown (NFC, trimmed, inner spaces collapsed) and its key (lower case, NFKC) that
/// must be unique in a pocket.
pub fn clean_name(raw: &str) -> Result<(String, String), NameError> {
    let shown: String = raw.nfc().filter(|c| !c.is_control() && !is_format(*c)).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    if shown.is_empty() {
        return Err(NameError::Empty);
    }
    if shown.chars().count() > NAME_MAX {
        return Err(NameError::TooLong);
    }
    let key: String = shown.nfkc().collect::<String>().to_lowercase();
    if blocked(&key) {
        return Err(NameError::NotAllowed);
    }
    Ok((shown, key))
}

/// Short guest text other guests see (team names, later answers on a big screen): same filter.
pub fn blocked(text: &str) -> bool {
    let lower = text.nfkc().collect::<String>().to_lowercase();
    if lower.contains("://") || lower.contains("www.") || lower.contains("http") {
        return true;
    }
    let plain: String = lower.chars().map(unleet).collect();
    let squashed: String = plain.chars().filter(|c| c.is_alphanumeric()).collect();
    if ANYWHERE.iter().any(|w| squashed.contains(w)) {
        return true;
    }
    plain.split(|c: char| !c.is_alphanumeric()).any(|word| WHOLE.contains(&word)) || WHOLE.contains(&squashed.as_str())
}

/// Why a guest's free text (a Santa question, an answer, a wish list) was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    Empty,
    TooLong(usize),
    NotAllowed,
}

impl TextError {
    pub fn message(&self) -> String {
        match self {
            TextError::Empty => "Type something first.".into(),
            TextError::TooLong(max) => format!("That's too long: at most {max} characters."),
            TextError::NotAllowed => "Please use other words.".into(),
        }
    }
}

/// Free text other guests read: NFC, control and invisible characters dropped, spaces
/// collapsed, at most `max` characters (counted before links come out, so the 201st character
/// is refused even if it was part of a link), links removed, then the word filter.
pub fn clean_text(raw: &str, max: usize) -> Result<String, TextError> {
    let flat: String = raw.nfc().map(|c| if c.is_control() { ' ' } else { c }).filter(|c| !is_format(*c)).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        return Err(TextError::TooLong(max));
    }
    let kept = flat.split(' ').filter(|w| !is_link(w)).collect::<Vec<_>>().join(" ");
    if kept.is_empty() {
        return Err(TextError::Empty);
    }
    if blocked_text(&kept) {
        return Err(TextError::NotAllowed);
    }
    Ok(kept)
}

/// The word filter for sentences: the same lists as [`blocked`], but checked word by word, so
/// "this hit song" isn't read as one squashed word. Dotted or leet spellings inside one word
/// ("s.h.1.t") are still caught.
pub fn blocked_text(text: &str) -> bool {
    let lower = text.nfkc().collect::<String>().to_lowercase();
    lower.split_whitespace().any(|word| {
        let plain: String = word.chars().map(unleet).collect();
        let squashed: String = plain.chars().filter(|c| c.is_alphanumeric()).collect();
        ANYWHERE.iter().any(|w| squashed.contains(w))
            || WHOLE.contains(&squashed.as_str())
            || plain.split(|c: char| !c.is_alphanumeric()).any(|part| WHOLE.contains(&part))
    })
}

/// Top-level domains a bare `name.tld` counts as a link with (with a `/` after it, any
/// alphabetic TLD does), so "e.g." or "size 8.5" stay.
const TLDS: &[&str] = &[
    "com", "net", "org", "io", "ly", "co", "uk", "de", "si", "me", "app", "shop", "info", "biz", "ru", "fr", "es", "it", "nl", "eu", "us", "ca",
    "au", "gg", "tv", "xyz", "link", "be", "ch", "at", "pl", "ua", "in", "jp", "cn", "br", "to", "dev", "ai", "gl", "page", "site", "store",
];

/// A word that is a link: has a scheme, starts with `www.`, or looks like `name.tld[/…]`.
pub fn is_link(word: &str) -> bool {
    let t = word.trim_matches(|c: char| "()[]{}<>\"'.,!?;:*".contains(c)).to_lowercase();
    if t.contains("://") || t.starts_with("www.") || t.starts_with("mailto:") || t.starts_with("tel:") {
        return true;
    }
    let (host, has_path) = match t.find(['/', '?', '#']) {
        Some(i) => (&t[..i], true),
        None => (t.as_str(), false),
    };
    let labels: Vec<&str> = host.split('.').collect();
    let Some(tld) = labels.last() else { return false };
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && (has_path || TLDS.contains(tld))
}

fn unleet(c: char) -> char {
    match c {
        '0' => 'o',
        '1' | '!' | '|' => 'i',
        '3' => 'e',
        '4' | '@' => 'a',
        '5' | '$' => 's',
        '7' => 't',
        '8' => 'b',
        _ => c,
    }
}

/// Zero-width and direction marks: invisible, so two names that look the same stay the same.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{00ad}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(clean_name("  Mia   Rose ").unwrap(), ("Mia Rose".into(), "mia rose".into()));
        assert_eq!(clean_name("MIA").unwrap().1, clean_name("mia").unwrap().1);
        assert_eq!(clean_name("Mi\u{200b}a").unwrap().0, "Mia");
        assert_eq!(clean_name("   "), Err(NameError::Empty));
        assert_eq!(clean_name(""), Err(NameError::Empty));
        assert_eq!(clean_name(&"a".repeat(21)), Err(NameError::TooLong));
        assert!(clean_name(&"é".repeat(20)).is_ok(), "characters, not bytes");
        for bad in ["fuckface", "F.U.C.K", "sh1t", "big dick", "Nazi", "http://x.io", "www.spam", "you cunt"] {
            assert!(clean_name(bad).is_err(), "{bad}");
        }
        assert_eq!(clean_name("a\u{0}b").unwrap().0, "ab");
        for ok in ["Dickens", "Scunthorpe", "Cockburn", "Jonas", "Priya", "Team Sexton", "Analyst"] {
            assert!(clean_name(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn free_text_drops_links_and_counts_characters() {
        assert_eq!(clean_text("  Shoe   size?\n", 200).unwrap(), "Shoe size?");
        assert_eq!(clean_text("see https://evil.example/x and www.spam.io or amazon.com/dp/1 bit.ly/x shop.si ok", 200).unwrap(), "see and or ok");
        assert_eq!(clean_text("(https://x.io/a), fine", 200).unwrap(), "fine");
        for keep in ["e.g. socks", "size 8.5", "i.e. warm", "Mr. Smith", "3.14"] {
            assert_eq!(clean_text(keep, 200).unwrap(), keep, "{keep}");
        }
        assert_eq!(clean_text(&"é".repeat(200), 200).unwrap().chars().count(), 200);
        assert_eq!(clean_text(&"é".repeat(201), 200), Err(TextError::TooLong(200)));
        assert_eq!(clean_text("https://only.link", 200), Err(TextError::Empty));
        assert_eq!(clean_text("   ", 200), Err(TextError::Empty));
        assert_eq!(clean_text("you are a fuckwit", 200), Err(TextError::NotAllowed));
        assert_eq!(clean_text("Do you like cocktails?", 200).unwrap(), "Do you like cocktails?");
        assert_eq!(clean_text("a\u{200b}b\u{0}c", 200).unwrap(), "ab c");
        assert_eq!(clean_text("This hit song", 200).unwrap(), "This hit song");
        for bad in ["sh1t gift", "s.h.i.t", "big dick energy", "F.U.C.K off"] {
            assert_eq!(clean_text(bad, 200), Err(TextError::NotAllowed), "{bad}");
        }
    }
}
