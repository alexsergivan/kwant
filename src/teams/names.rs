//! Bot names and team handles for a ready-made team (spec T9, design A3): what is valid, what
//! counts as taken, and the next free one (`Drafter` → `Drafter2`, `inbox` → `inbox2`).

/// Longest bot name or handle.
pub const MAX_LEN: usize = 40;

/// A bot name as the app accepts one: not empty, at most [`MAX_LEN`] characters, no whitespace
/// and no `@` (the same rule as creating a bot by hand).
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=MAX_LEN).contains(&n) && !name.chars().any(|c| c.is_whitespace() || c == '@' || c.is_control())
}

/// A team handle: a lower-case slug, `[a-z0-9-]`, starting with a letter or digit, at most
/// [`MAX_LEN`] characters. The team is called this and its chat is `team-<handle>`.
pub fn valid_handle(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= MAX_LEN
        && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && h.as_bytes()[0] != b'-'
}

/// A handle from any text (a folder or repo name, spec D Q-D5): lower case, every run of other
/// characters becomes one `-`, trimmed and cut to fit. `None` when nothing is left.
pub fn slug(text: &str) -> Option<String> {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out = out.trim_matches('-').to_string();
    out.truncate(MAX_LEN - 3);
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

/// `base`, else the next free one (ignoring case): a name that ends in a number counts on from
/// it (`TechLead4` → `TechLead5`, `Agent007` → `Agent008`), any other gets `2`, `3`, …
/// (`Drafter` → `Drafter2`). Each candidate is cut so the number still fits in [`MAX_LEN`].
pub fn next_free(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let digits = base.len() - base.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (stem, first, width) = match base[base.len() - digits..].parse::<u64>() {
        Ok(n) if digits > 0 && n < u64::MAX / 2 => (&base[..base.len() - digits], n + 1, digits),
        _ => (base, 2, 0),
    };
    for n in first.. {
        let suffix = format!("{n:0width$}");
        let keep: String = stem.chars().take(MAX_LEN.saturating_sub(suffix.len())).collect();
        let candidate = format!("{keep}{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("Drafter"));
        assert!(valid_name("Drafter2"));
        for bad in ["", "Reply Drafter", "@Drafter", "Drafter\n", &"x".repeat(41)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn handles() {
        assert!(valid_handle("inbox"));
        assert!(valid_handle("inbox-team2"));
        for bad in ["", "Inbox", "inbox team", "-inbox", "@inbox", "inbox_", &"a".repeat(41)] {
            assert!(!valid_handle(bad), "{bad:?}");
        }
        assert_eq!(slug("My App!").as_deref(), Some("my-app"));
        assert_eq!(slug("--shop.web--").as_deref(), Some("shop-web"));
        assert_eq!(slug("ëëë"), None);
        assert!(valid_handle(&slug(&"a".repeat(80)).unwrap()));
    }

    #[test]
    fn bumps_without_a_space() {
        let taken = ["drafter", "drafter2"];
        let is = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
        assert_eq!(next_free("Drafter", is), "Drafter3");
        assert_eq!(next_free("Researcher", is), "Researcher");
        let long = "a".repeat(40);
        let got = next_free(&long, |n| n == long);
        assert_eq!(got.len(), 40);
        assert!(got.ends_with('2'));
    }

    #[test]
    fn a_name_ending_in_a_number_counts_on() {
        let taken = ["techlead4", "techlead5", "agent007", "inbox-2"];
        let is = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
        assert_eq!(next_free("TechLead4", is), "TechLead6");
        assert_eq!(next_free("TechLead9", |n| n == "TechLead9"), "TechLead10");
        assert_eq!(next_free("Agent007", is), "Agent008");
        assert_eq!(next_free("inbox-2", is), "inbox-3");
        assert_eq!(next_free("42", |n| n == "42"), "43");
        // too big to count on: append as before
        let huge = "Bot99999999999999999999999";
        assert_eq!(next_free(huge, |n| n == huge), format!("{huge}2"));
        let long = format!("{}7", "a".repeat(39));
        let got = next_free(&long, |n| n == long);
        assert_eq!(got.len(), 40);
        assert!(got.ends_with('8'));
    }
}
