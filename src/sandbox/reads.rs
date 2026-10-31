//! Cheaper page reads for bots: a hash of what a read shows, so a read of an unchanged page can
//! answer "same as last time" instead of the whole page again, and a cap on the controls listed,
//! with the ones matching the bot's own words first (plain word matching, no model call).

/// The most controls one read lists.
pub const SHOWN_CONTROLS: usize = 40;
/// The most controls read from the page before picking (the hash covers all of them).
pub const READ_CONTROLS: usize = 400;
/// Bots remembered per browser for "unchanged since your last read".
pub const READERS_KEPT: usize = 32;

/// FNV-1a, 64 bits: stable across runs and builds, unlike `DefaultHasher`.
fn fnv1a(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.bytes().chain(std::iter::once(0xff)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// What a read shows, as a short hex string: the address, title, text, every control (not only
/// the ones listed) and the `find` words that picked them.
pub fn page_hash(url: &str, title: &str, text: &str, controls: &[String], find: &str) -> String {
    let mut parts = vec![url, title, text, find];
    parts.extend(controls.iter().map(String::as_str));
    format!("{:016x}", fnv1a(&parts))
}

/// The words of `find` worth matching: lowercase, 3+ letters, without a few filler words.
pub fn find_words(find: &str) -> Vec<String> {
    const FILLER: &[&str] = &["the", "and", "for", "with", "this", "that", "from", "into", "button", "link"];
    let mut words: Vec<String> = Vec::new();
    for w in find.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase) {
        if w.chars().count() >= 3 && !FILLER.contains(&w.as_str()) && !words.contains(&w) {
            words.push(w);
        }
    }
    words
}

/// Picks at most `cap` of the page's control lines. Without `find` words they keep page order
/// (topmost first). With them, the lines naming the most of those words come first, ties in page
/// order. Ref numbers are part of each line, so they stay what the page gave them. The note says
/// what was left out, for the bot to ask again with other words.
pub fn pick_controls(lines: &[String], find: &str, cap: usize) -> (Vec<String>, Option<String>) {
    let words = find_words(find);
    let mut order: Vec<(usize, &String)> = lines.iter().map(|l| (0, l)).collect();
    let mut matched = 0;
    if !words.is_empty() {
        for (score, line) in order.iter_mut() {
            let low = line.to_lowercase();
            *score = words.iter().filter(|w| low.contains(w.as_str())).count();
            if *score > 0 {
                matched += 1;
            }
        }
        // Stable: equal scores keep page order
        order.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    }
    let shown: Vec<String> = order.iter().take(cap).map(|(_, l)| (*l).clone()).collect();
    let total = lines.len();
    let note = match (words.is_empty(), matched) {
        (false, 0) => Some(format!("No control matches \"{}\"; showing them in page order.", find.trim())),
        _ if total <= cap => None,
        (true, _) => Some(format!(
            "Showing the first {cap} of {total} controls. Read the page again with find (words for what you look for) to list the matching ones first."
        )),
        (false, m) => Some(format!(
            "Showing {cap} of {total} controls: the {} matching \"{}\" first.",
            m.min(cap),
            find.trim()
        )),
    };
    (shown, note)
}

/// Remembers which read each bot (`reader`) saw last, and answers whether a new read is the same.
#[derive(Debug, Default)]
pub struct Reads {
    seen: Vec<(String, String)>,
}

impl Reads {
    /// Whether `reader`'s last read had this `hash`.
    pub fn is_same(&self, reader: &str, hash: &str) -> bool {
        self.seen.iter().any(|(r, h)| r == reader && h == hash)
    }

    /// `reader` was just shown the read with this `hash`.
    pub fn saw(&mut self, reader: &str, hash: &str) {
        self.seen.retain(|(r, _)| r != reader);
        if self.seen.len() >= READERS_KEPT {
            self.seen.remove(0);
        }
        self.seen.push((reader.to_string(), hash.to_string()));
    }

    /// `reader` must get the whole page next time (it asked for it, or lost it).
    pub fn forget(&mut self, reader: &str) {
        self.seen.retain(|(r, _)| r != reader);
    }
}

/// What a read of an unchanged page says instead of the page.
pub const UNCHANGED_NOTE: &str = "Same as your last read of this page: nothing on it changed, so its text and the numbered \
     controls you were shown still hold. Read it with full=true if you need it again.";

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(names: &[&str]) -> Vec<String> {
        names.iter().enumerate().map(|(i, n)| format!("[{}] button \"{n}\"", i + 1)).collect()
    }

    #[test]
    fn the_hash_changes_with_anything_a_read_shows() {
        let c = lines(&["Add to cart"]);
        let h = page_hash("u", "t", "text", &c, "");
        assert_eq!(h, page_hash("u", "t", "text", &c, ""), "stable");
        assert_eq!(h.len(), 16);
        assert_ne!(h, page_hash("u2", "t", "text", &c, ""));
        assert_ne!(h, page_hash("u", "t2", "text", &c, ""));
        assert_ne!(h, page_hash("u", "t", "text!", &c, ""));
        assert_ne!(h, page_hash("u", "t", "text", &lines(&["Remove"]), ""));
        assert_ne!(h, page_hash("u", "t", "text", &c, "cart"));
        // Parts don't run into each other
        assert_ne!(page_hash("ab", "c", "", &[], ""), page_hash("a", "bc", "", &[], ""));
    }

    #[test]
    fn find_keeps_the_words_that_mean_something() {
        assert_eq!(find_words("Add the Bike Light to cart"), vec!["add", "bike", "light", "cart"]);
        assert_eq!(find_words("  "), Vec::<String>::new());
        assert_eq!(find_words("checkout button, checkout"), vec!["checkout"]);
    }

    #[test]
    fn a_short_list_is_shown_whole_and_in_page_order() {
        let all = lines(&["Menu", "Add to cart", "Checkout"]);
        assert_eq!(pick_controls(&all, "", 40), (all.clone(), None));
    }

    #[test]
    fn a_long_list_is_cut_and_matching_controls_come_first() {
        let names: Vec<String> = (1..=60).map(|i| if i == 55 { "Bike Light".into() } else { format!("Item {i}") }).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let all = lines(&names);
        let (shown, note) = pick_controls(&all, "", 40);
        assert_eq!(shown.len(), 40);
        assert_eq!(shown[0], "[1] button \"Item 1\"");
        assert!(note.unwrap().starts_with("Showing the first 40 of 60 controls."));

        let (shown, note) = pick_controls(&all, "bike light", 40);
        assert_eq!(shown[0], "[55] button \"Bike Light\"", "keeps its ref");
        assert_eq!(shown[1], "[1] button \"Item 1\"", "then page order");
        assert_eq!(note.unwrap(), "Showing 40 of 60 controls: the 1 matching \"bike light\" first.");

        // More words matched ranks higher
        let all = lines(&["Light", "Bike", "Bike Light"]);
        let (shown, note) = pick_controls(&all, "bike light", 40);
        assert_eq!(shown[0], "[3] button \"Bike Light\"");
        assert_eq!(&shown[1..], &all[..2]);
        assert_eq!(note, None);

        let (shown, note) = pick_controls(&all, "zebra", 2);
        assert_eq!(shown, all[..2]);
        assert_eq!(note.unwrap(), "No control matches \"zebra\"; showing them in page order.");
    }

    #[test]
    fn each_bot_is_told_unchanged_only_about_its_own_last_read() {
        let mut r = Reads::default();
        assert!(!r.is_same("a", "h1"));
        r.saw("a", "h1");
        assert!(r.is_same("a", "h1"));
        assert!(!r.is_same("b", "h1"), "another bot never saw it");
        r.saw("a", "h2");
        assert!(!r.is_same("a", "h1"));
        r.forget("a");
        assert!(!r.is_same("a", "h2"));
        for i in 0..READERS_KEPT + 5 {
            r.saw(&format!("bot{i}"), "h");
        }
        assert_eq!(r.seen.len(), READERS_KEPT);
        assert!(r.is_same(&format!("bot{}", READERS_KEPT + 4), "h"));
        assert!(!r.is_same("bot0", "h"));
    }
}
