//! Cited reads: a bot quotes what it read, and kwant checks the words are really on the page and
//! gives back a link that opens the page with them highlighted (a `#:~:text=` text fragment).
//! Pure, so the rules are tested as tables; `routes::cite_sandbox` reads the page's text (and its
//! same-origin frames') and calls [`cite_text`] or [`cite_block`]. Nothing here is made up: a
//! quote that isn't on the page word for word, or is on it more than once, is refused.

/// One document's rendered text: the page itself, or a frame inside it with its own address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameText {
    pub url: String,
    pub text: String,
}

/// A checked quote: the page's own words, the address they are on, and a link that opens it
/// with them highlighted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Citation {
    pub quote: String,
    pub url: String,
    pub link: String,
}

/// The longest quote cited in one text fragment; a longer one (or one over several lines) is cited
/// by its first and last words.
const EXACT_MAX_CHARS: usize = 300;
/// Words at each end of a long quote's link, and the most words of context around it.
const EDGE_WORDS: usize = 5;
const CONTEXT_WORDS: usize = 3;
/// Characters of a block shown in a page read; citing it copies the whole block.
pub const BLOCK_SHOWN_CHARS: usize = 300;

/// A page's text as lines, each with its runs of whitespace made one space; empty lines dropped.
/// A line is roughly one block of the page (a paragraph, a heading, a list item, a cell).
pub fn page_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect()
}

fn normalize(quote: &str) -> String {
    quote.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\n'
}

/// Where `pat` matches `hay` at `at` (a space in `pat` matches a space or a line break), and
/// where the match ends.
fn match_at(hay: &[char], at: usize, pat: &[char], fold: bool) -> Option<usize> {
    if pat.is_empty() || at + pat.len() > hay.len() {
        return None;
    }
    for (i, &p) in pat.iter().enumerate() {
        let h = hay[at + i];
        let same = if p == ' ' { is_space(h) } else if fold { lower(h) == p } else { h == p };
        if !same {
            return None;
        }
    }
    Some(at + pat.len())
}

fn skip_space(hay: &[char], mut at: usize) -> usize {
    while at < hay.len() && is_space(hay[at]) {
        at += 1;
    }
    at
}

/// A text fragment: `text=[prefix-,]start[,end][,-suffix]`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fragment {
    prefix: Option<String>,
    start: String,
    end: Option<String>,
    suffix: Option<String>,
}

fn folded(s: &str) -> Vec<char> {
    s.chars().map(lower).collect()
}

/// What a browser highlights for `f` on a page whose text is `hay`: the first match, compared
/// without case. It is more lenient than a browser (no word-boundary or block rules), so it finds
/// every match a browser would and maybe more; a link is only given when the first is the quote.
fn resolve(hay: &[char], f: &Fragment) -> Option<(usize, usize)> {
    let start = folded(&f.start);
    let prefix = f.prefix.as_deref().map(folded);
    let end = f.end.as_deref().map(folded);
    let suffix = f.suffix.as_deref().map(folded);
    let suffix_ok = |at: usize| match &suffix {
        Some(sfx) => match_at(hay, skip_space(hay, at), sfx, true).is_some(),
        None => true,
    };
    for p in 0..hay.len() {
        let s = match &prefix {
            Some(pre) => match match_at(hay, p, pre, true) {
                Some(pe) => skip_space(hay, pe),
                None => continue,
            },
            None => p,
        };
        let Some(se) = match_at(hay, s, &start, true) else { continue };
        match &end {
            None if suffix_ok(se) => return Some((s, se)),
            None => {}
            Some(end) => {
                for q in se..hay.len() {
                    if let Some(qe) = match_at(hay, q, end, true) {
                        if suffix_ok(qe) {
                            return Some((s, qe));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Percent-encodes a fragment part: everything but ASCII letters, digits and `._~`, so the
/// `-`, `,` and `&` that structure a text fragment are never taken literally.
fn encode(part: &str) -> String {
    let mut out = String::new();
    for b in part.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn fragment_param(f: &Fragment) -> String {
    let mut t = String::from("text=");
    if let Some(p) = &f.prefix {
        t.push_str(&format!("{}-,", encode(p)));
    }
    t.push_str(&encode(&f.start));
    if let Some(e) = &f.end {
        t.push_str(&format!(",{}", encode(e)));
    }
    if let Some(s) = &f.suffix {
        t.push_str(&format!(",-{}", encode(s)));
    }
    t
}

/// `url` with the text fragment added (after any fragment the address already has, as in
/// `#/route:~:text=…`; an older text fragment is replaced).
fn link_for(url: &str, f: &Fragment) -> String {
    let base = match url.find(":~:") {
        Some(i) => &url[..i],
        None => url,
    };
    let sep = if base.contains('#') { ":~:" } else { "#:~:" };
    format!("{base}{sep}{}", fragment_param(f))
}

/// Word spans (start, end) in `hay[from..to]`.
fn words(hay: &[char], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = from;
    while i < to {
        if is_space(hay[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < to && !is_space(hay[i]) {
            i += 1;
        }
        out.push((s, i));
    }
    out
}

fn text_of(hay: &[char], s: usize, e: usize) -> String {
    hay[s..e].iter().map(|&c| if c == '\n' { ' ' } else { c }).collect()
}

/// The line `at` is on: (first char, end).
fn line_around(hay: &[char], at: usize) -> (usize, usize) {
    let s = hay[..at].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
    let e = hay[at..].iter().position(|&c| c == '\n').map_or(hay.len(), |i| at + i);
    (s, e)
}

/// A text fragment whose first match is exactly `hay[s..e]`, adding words before and after it on
/// the same line until it is; `None` when no fragment does.
fn fragment_for(hay: &[char], s: usize, e: usize) -> Option<Fragment> {
    let span_words = words(hay, s, e);
    let multi_line = hay[s..e].contains(&'\n');
    let mut shapes: Vec<(String, Option<String>)> = Vec::new();
    if !multi_line && e - s <= EXACT_MAX_CHARS {
        shapes.push((text_of(hay, s, e), None));
    }
    // First and last words, each kept within its own line
    let (first_line_end, last_line_start) = (line_around(hay, s).1, line_around(hay, e.saturating_sub(1)).0);
    let head: Vec<_> = span_words.iter().filter(|w| w.1 <= first_line_end).take(EDGE_WORDS).collect();
    let tail: Vec<_> = span_words.iter().filter(|w| w.0 >= last_line_start).collect();
    let tail = &tail[tail.len().saturating_sub(EDGE_WORDS)..];
    if let (Some(h0), Some(h1), Some(t0), Some(t1)) = (head.first(), head.last(), tail.first(), tail.last()) {
        if h1.1 <= t0.0 {
            shapes.push((text_of(hay, h0.0, h1.1), Some(text_of(hay, t0.0, t1.1))));
        }
    }
    let (line_s, _) = line_around(hay, s);
    let (_, line_e) = line_around(hay, e.saturating_sub(1));
    let before = words(hay, line_s, s);
    let after = words(hay, e, line_e);
    let prefix = |k: usize| {
        let w = &before[before.len().saturating_sub(k)..];
        (k > 0 && !w.is_empty()).then(|| text_of(hay, w[0].0, w[w.len() - 1].1))
    };
    let suffix = |k: usize| {
        let w = &after[..k.min(after.len())];
        (k > 0 && !w.is_empty()).then(|| text_of(hay, w[0].0, w[w.len() - 1].1))
    };
    for (start, end) in shapes {
        // Fewest words first: none, then words before, after, both
        let mut tries = vec![(None, None)];
        for k in 1..=CONTEXT_WORDS {
            tries.extend([(prefix(k), None), (None, suffix(k)), (prefix(k), suffix(k))]);
        }
        for (prefix, suffix) in tries {
            let f = Fragment { prefix, start: start.clone(), end: end.clone(), suffix };
            if resolve(hay, &f) == Some((s, e)) {
                return Some(f);
            }
        }
    }
    None
}

fn cite_at(url: &str, hay: &[char], s: usize, e: usize) -> Result<Citation, String> {
    let quote = text_of(hay, s, e);
    let f = fragment_for(hay, s, e).ok_or_else(|| {
        "Couldn't make a link that points only at this passage (the page repeats it in the same words); \
         quote a longer or different passage."
            .to_string()
    })?;
    Ok(Citation { quote, url: url.to_string(), link: link_for(url, &f) })
}

fn joined(text: &str) -> Vec<char> {
    page_lines(text).join("\n").chars().collect()
}

/// Every place `pat` is in `hay`, as start positions.
fn all_matches(hay: &[char], pat: &[char], fold: bool) -> Vec<usize> {
    (0..hay.len()).filter(|&p| match_at(hay, p, pat, fold).is_some()).collect()
}

fn mid_word(hay: &[char], at: usize) -> bool {
    at > 0 && at < hay.len() && hay[at - 1].is_alphanumeric() && hay[at].is_alphanumeric()
}

/// Cites `quote` from the page: it must be on it word for word (runs of whitespace and line
/// breaks count as one space), once, in whole words, in the page or one of its frames
/// (`frames[0]` is the page). The answer's quote is the page's own text.
pub fn cite_text(frames: &[FrameText], quote: &str) -> Result<Citation, String> {
    let q = normalize(quote);
    if q.is_empty() {
        return Err("Give the exact words to cite in text, or a block id in block.".into());
    }
    let pat: Vec<char> = q.chars().collect();
    let pages: Vec<(&str, Vec<char>)> = frames.iter().map(|f| (f.url.as_str(), joined(&f.text))).collect();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for (i, (_, hay)) in pages.iter().enumerate() {
        found.extend(all_matches(hay, &pat, false).into_iter().map(|p| (i, p)));
    }
    match found.as_slice() {
        [] => {
            let fold: Vec<char> = pat.iter().map(|&c| lower(c)).collect();
            let near = pages.iter().find_map(|(_, hay)| all_matches(hay, &fold, true).first().map(|&p| text_of(hay, p, p + pat.len())));
            Err(match near {
                Some(near) => format!("Not cited: the page has these words with other capitals (\"{near}\"). Quote them exactly as the page has them."),
                None => "Not cited: these words aren't on the page as given. Quote the page word for word (copy them from \
                         your read), or cite a block by its id; never quote from memory."
                    .into(),
            })
        }
        [(i, p)] => {
            let (url, hay) = &pages[*i];
            let e = p + pat.len();
            if mid_word(hay, *p) || mid_word(hay, e) {
                return Err("Not cited: the quote starts or ends in the middle of a word. Quote whole words.".into());
            }
            cite_at(url, hay, *p, e)
        }
        many => Err(format!(
            "Not cited: these words are on the page {} times. Quote a longer passage, so it is only there once.",
            many.len()
        )),
    }
}

/// FNV-1a of a block's text, 4 hex digits: part of its id, so an id from an older read never
/// cites a different block.
fn tag(line: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in line.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{:04x}", h & 0xffff)
}

/// The page's blocks (its lines) with their ids, `b<number>-<tag>`.
pub fn blocks(text: &str) -> Vec<(String, String)> {
    page_lines(text).into_iter().enumerate().map(|(i, l)| (format!("b{}-{}", i + 1, tag(&l)), l)).collect()
}

/// A block as a page read shows it: `[b3-9f2a] text`, long ones cut short.
pub fn block_line(id: &str, text: &str) -> String {
    let mut shown: String = text.chars().take(BLOCK_SHOWN_CHARS).collect();
    if text.chars().count() > BLOCK_SHOWN_CHARS {
        shown.push('…');
    }
    format!("[{id}] {shown}")
}

/// Cites a whole block of the page by the id a read showed (`b3-9f2a`, with or without the
/// brackets). If the page moved, the same block is found by its text; if it is gone, refused.
pub fn cite_block(page: &FrameText, id: &str) -> Result<Citation, String> {
    let id = id.trim().trim_start_matches('[').trim_end_matches(']');
    let parsed = id.strip_prefix('b').and_then(|r| r.split_once('-')).and_then(|(n, t)| Some((n.parse::<usize>().ok()?, t)));
    let Some((n, t)) = parsed else {
        return Err(format!("\"{id}\" isn't a block id; they look like b3-9f2a, from a page read with blocks."));
    };
    let lines = page_lines(&page.text);
    let index = match lines.get(n.wrapping_sub(1)) {
        Some(l) if tag(l) == t => Some(n - 1),
        _ => {
            let same: Vec<usize> = (0..lines.len()).filter(|&i| tag(&lines[i]) == t).collect();
            (same.len() == 1).then(|| same[0])
        }
    };
    let Some(index) = index else {
        return Err(format!("Not cited: block {id} isn't on the page any more (it changed). Read the page again with blocks."));
    };
    let hay: Vec<char> = lines.join("\n").chars().collect();
    let s: usize = lines[..index].iter().map(|l| l.chars().count() + 1).sum();
    cite_at(&page.url, &hay, s, s + lines[index].chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(text: &str) -> Vec<FrameText> {
        vec![FrameText { url: "https://example.org/a".into(), text: text.into() }]
    }

    #[test]
    fn a_quote_on_the_page_gets_a_link_that_highlights_it() {
        let c = cite_text(&page("Intro\nThe  quick brown\tfox jumps.\nEnd"), "quick   brown fox").unwrap();
        assert_eq!(c.quote, "quick brown fox");
        assert_eq!(c.url, "https://example.org/a");
        assert_eq!(c.link, "https://example.org/a#:~:text=quick%20brown%20fox");
    }

    #[test]
    fn the_link_text_is_encoded() {
        let c = cite_text(&page("Price: 5-10 apples, & pears #1 (ok) über"), "5-10 apples, & pears #1 (ok) über").unwrap();
        assert_eq!(c.link, "https://example.org/a#:~:text=5%2D10%20apples%2C%20%26%20pears%20%231%20%28ok%29%20%C3%BCber");
    }

    #[test]
    fn an_address_with_a_fragment_keeps_it() {
        let f = Fragment { prefix: None, start: "x".into(), end: None, suffix: None };
        assert_eq!(link_for("https://s.test/#/route", &f), "https://s.test/#/route:~:text=x");
        assert_eq!(link_for("https://s.test/p#:~:text=old", &f), "https://s.test/p#:~:text=x");
    }

    #[test]
    fn what_isnt_on_the_page_once_and_in_whole_words_is_refused() {
        let p = page("Cats sleep a lot.\nDogs bark.\nCats sleep a lot.");
        assert!(cite_text(&p, "Cats fly").unwrap_err().contains("aren't on the page"));
        assert!(cite_text(&p, "cats SLEEP").unwrap_err().contains("other capitals (\"Cats sleep\")"));
        assert!(cite_text(&p, "Cats sleep a lot").unwrap_err().contains("2 times"));
        assert!(cite_text(&p, "ogs bark").unwrap_err().contains("middle of a word"));
        assert!(cite_text(&p, "  ").is_err());
        // The quote is the page's text, not the bot's
        assert_eq!(cite_text(&p, "Dogs\n bark.").unwrap().quote, "Dogs bark.");
    }

    #[test]
    fn words_found_earlier_in_other_case_or_inside_a_word_get_context() {
        // The browser matches without case, so the earlier "The report" would be highlighted
        let c = cite_text(&page("The report below is late.\nSee the report below."), "the report below").unwrap();
        assert!(c.link.ends_with("#:~:text=See-,the%20report%20below"), "{}", c.link);
        let c = cite_text(&page("Categories\nA cat sat"), "cat sat").unwrap();
        assert!(c.link.ends_with("#:~:text=A-,cat%20sat") || c.link.ends_with("#:~:text=cat%20sat"), "{}", c.link);
        // Whatever the shape, the browser's first match is the quote
        let hay: Vec<char> = "Categories\nA cat sat".chars().collect();
        assert_eq!(resolve(&hay, &fragment_for(&hay, 13, 20).unwrap()), Some((13, 20)));
    }

    #[test]
    fn a_quote_over_several_lines_or_a_long_one_is_cited_by_its_ends() {
        let c = cite_text(&page("Title\nFirst para ends here.\nSecond para starts here and goes on."), "ends here. Second para starts").unwrap();
        assert_eq!(c.quote, "ends here. Second para starts");
        assert!(c.link.ends_with("#:~:text=ends%20here.,Second%20para%20starts"), "{}", c.link);
        let long: Vec<String> = (1..=80).map(|i| format!("word{i}")).collect();
        let c = cite_text(&page(&long.join(" ")), &long[2..70].join(" ")).unwrap();
        assert!(c.link.ends_with("#:~:text=word3%20word4%20word5%20word6%20word7,word66%20word67%20word68%20word69%20word70"), "{}", c.link);
    }

    #[test]
    fn a_quote_in_a_frame_links_to_the_frame() {
        let frames = vec![
            FrameText { url: "https://shop.test/".into(), text: "Shop".into() },
            FrameText { url: "https://shop.test/reviews".into(), text: "Great light, very bright.".into() },
        ];
        let c = cite_text(&frames, "very bright").unwrap();
        assert_eq!(c.url, "https://shop.test/reviews");
        assert_eq!(c.link, "https://shop.test/reviews#:~:text=very%20bright");
        let frames = vec![frames[1].clone(), FrameText { url: "https://x.test/".into(), text: "very bright".into() }];
        assert!(cite_text(&frames, "very bright").unwrap_err().contains("2 times"), "counted across frames");
    }

    #[test]
    fn a_block_is_cited_by_its_id_even_after_the_page_moved() {
        let p = FrameText { url: "https://example.org/a".into(), text: "Heading\n\nBody text here.\nFooter".into() };
        let b = blocks(&p.text);
        assert_eq!(b.len(), 3);
        assert!(b[1].0.starts_with("b2-") && b[1].1 == "Body text here.");
        let c = cite_block(&p, &format!("[{}]", b[1].0)).unwrap();
        assert_eq!(c.quote, "Body text here.");
        assert_eq!(c.link, "https://example.org/a#:~:text=Body%20text%20here.");
        // A new line above: the same id still finds it
        let moved = FrameText { text: format!("Banner\n{}", p.text), ..p.clone() };
        assert_eq!(cite_block(&moved, &b[1].0).unwrap().quote, "Body text here.");
        // Gone, or not an id
        let gone = FrameText { text: "Heading\nOther".into(), ..p.clone() };
        assert!(cite_block(&gone, &b[1].0).unwrap_err().contains("isn't on the page any more"));
        assert!(cite_block(&p, "3").unwrap_err().contains("isn't a block id"));
        // A block whose words come earlier on the page (in any case) can't be linked on its own
        let rep = FrameText { url: "https://e.test/".into(), text: "YES\nAre you sure\nYes".into() };
        let id = &blocks(&rep.text)[2].0;
        assert!(cite_block(&rep, id).unwrap_err().contains("Couldn't make a link"));
        assert!(block_line("b1-0000", &"x".repeat(400)).ends_with("x…"));
    }

    #[test]
    fn a_passage_that_cant_be_singled_out_is_refused() {
        // Two identical lines: the second can't be told apart by its own line's words
        let rep = FrameText { url: "u".into(), text: "Same line\nSame line".into() };
        let id = &blocks(&rep.text)[1].0;
        assert!(cite_block(&rep, id).unwrap_err().contains("Couldn't make a link"));
    }
}
