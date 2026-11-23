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
}
