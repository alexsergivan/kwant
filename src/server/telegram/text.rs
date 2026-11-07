//! What a bot wrote, as Telegram HTML (design G4): bold, italic, strike, code, code blocks,
//! http(s) and tg links, headings as bold, bullets as "•". Everything else is escaped text, so
//! Telegram never fails to parse it (test plan G-T6). Long answers are split at line breaks into
//! parts of at most 4,096 UTF-16 units each, counted on the HTML itself (stricter than Telegram,
//! which counts after parsing), each ending "(1/2)"…; a code block that is split is closed and
//! opened again (G-T5).

/// Telegram's limit for one message.
pub const MAX_UNITS: usize = 4096;
/// Room kept for "\n(10/10)" and a `</pre>` / `<pre>` pair around a split code block.
const RESERVE: usize = 32;

pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// A link target we keep: http(s) or tg, no spaces or quotes.
fn safe_url(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://") || l.starts_with("tg://")) && !u.chars().any(|c| c.is_whitespace() || c == '"' || c == '<' || c == '>')
}

fn find_from(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if pat.is_empty() || chars.len() < pat.len() {
        return None;
    }
    (from..=chars.len() - pat.len()).find(|&i| chars[i..i + pat.len()] == *pat)
}

/// One line's inline markup.
pub fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
    'chars: while i < chars.len() {
        let c = chars[i];
        // `code`
        if c == '`' {
            if let Some(j) = find_from(&chars, i + 1, &['`']).filter(|&j| j > i + 1) {
                out.push_str("<code>");
                out.push_str(&escape(&chars[i + 1..j].iter().collect::<String>()));
                out.push_str("</code>");
                i = j + 1;
                continue;
            }
        }
        // **bold** and ~~strike~~
        for (pat, tag) in [(['*', '*'], "b"), (['~', '~'], "s")] {
            if chars[i..].starts_with(&pat) {
                if let Some(j) = find_from(&chars, i + 2, &pat).filter(|&j| j > i + 2 && !chars[i + 2].is_whitespace()) {
                    let inner: String = chars[i + 2..j].iter().collect();
                    out.push_str(&format!("<{tag}>{}</{tag}>", inline(&inner)));
                    i = j + 2;
                    continue 'chars;
                }
            }
        }
        // *italic* or _italic_, not inside a word (snake_case stays as it is)
        if (c == '*' || c == '_') && !word(i.checked_sub(1).and_then(|p| chars.get(p))) {
            if let Some(j) = find_from(&chars, i + 1, &[c]).filter(|&j| j > i + 1 && !chars[i + 1].is_whitespace() && !chars[j - 1].is_whitespace() && !word(chars.get(j + 1))) {
                let inner: String = chars[i + 1..j].iter().collect();
                out.push_str(&format!("<i>{}</i>", inline(&inner)));
                i = j + 1;
                continue;
            }
        }
        // [text](url)
        if c == '[' {
            if let Some(close) = find_from(&chars, i + 1, &[']', '(']) {
                let mut depth = 0i32;
                let mut end = None;
                for (k, ch) in chars.iter().enumerate().skip(close + 2) {
                    match ch {
                        '(' => depth += 1,
                        ')' if depth == 0 => {
                            end = Some(k);
                            break;
                        }
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                if let Some(end) = end {
                    let label: String = chars[i + 1..close].iter().collect();
                    let url: String = chars[close + 2..end].iter().collect();
                    if safe_url(&url) && !label.trim().is_empty() {
                        out.push_str(&format!("<a href=\"{}\">{}</a>", escape(&url), escape(&label)));
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push_str(&escape(&c.to_string()));
        i += 1;
    }
    out
}

/// A line of the rendered message.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    html: String,
    /// Inside a code block (rendered as escaped text, between `<pre>` and `</pre>`).
    code: bool,
}

fn lines(md: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in md.replace("\r\n", "\n").split('\n') {
        let t = raw.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Line { html: escape(raw), code: true });
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        let html = if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            format!("<b>{}</b>", inline(t[hashes..].trim()))
        } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            let indent = raw.len() - t.len();
            format!("{}• {}", " ".repeat(indent), inline(rest))
        } else if let Some(rest) = t.strip_prefix("> ") {
            format!("▎ {}", inline(rest))
        } else {
            inline(raw)
        };
        out.push(Line { html, code: false });
    }
    out
}

/// `text` cut into pieces once escaped: the first at most `first` units, the rest at most
/// `budget`. A piece ends after a space when one falls in its second half, else mid-word.
fn chop(text: &str, first: usize, budget: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    let mut limit = first.max(1);
    // Byte length and units of `cur` up to and including its last space
    let mut space: Option<(usize, usize)> = None;
    for c in text.chars() {
        let e = escape(&c.to_string());
        let n = utf16_len(&e);
        if used + n > limit && !cur.is_empty() {
            match space {
                Some((b, u)) if u * 2 >= limit => {
                    let rest = cur.split_off(b);
                    parts.push(cur);
                    cur = rest;
                    used -= u;
                }
                _ => {
                    parts.push(std::mem::take(&mut cur));
                    used = 0;
                }
            }
            space = None;
            limit = budget;
        }
        cur.push_str(&e);
        used += n;
        if c == ' ' {
            space = Some((cur.len(), used));
        }
    }
    if !cur.is_empty() || parts.is_empty() {
        parts.push(cur);
    }
    parts
}

/// The message as parts ready to send: `header` (HTML, e.g. "<b>Drafter</b> · #general") on the
/// first, then the body. One part has no suffix; more each end "(i/n)".
pub fn message_parts(header: Option<&str>, md: &str) -> Vec<String> {
    let budget = MAX_UNITS - RESERVE;
    let mut bodies: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_code = false;
    let mut used = 0;
    if let Some(h) = header {
        cur.push_str(h);
        used = utf16_len(h);
    }
    let mut first_line = header.is_none();
    let mut queue: std::collections::VecDeque<Line> = lines(md).into();
    while let Some(l) = queue.pop_front() {
        if utf16_len(&l.html) > budget {
            // Too long for any part: several plain lines (inline markup dropped), the first
            // filling what is left of this part when that is a fair amount
            let room = budget.saturating_sub(used + 1 + "<pre></pre></pre>".len());
            let first = if room >= budget / 4 { room } else { budget };
            let raw = unescape(&strip_tags(&l.html));
            for (i, html) in chop(&raw, first, budget).into_iter().enumerate() {
                queue.insert(i, Line { html, code: l.code });
            }
            continue;
        }
        let mut piece = String::new();
        if !first_line {
            piece.push('\n');
        }
        if l.code && !cur_code {
            piece.push_str("<pre>");
        }
        if !l.code && cur_code {
            piece = format!("</pre>{piece}");
        }
        piece.push_str(&l.html);
        let n = utf16_len(&piece);
        if used + n > budget && used > 0 {
            if cur_code {
                cur.push_str("</pre>");
            }
            bodies.push(std::mem::take(&mut cur));
            used = 0;
            piece = String::new();
            if l.code {
                piece.push_str("<pre>");
            }
            piece.push_str(&l.html);
            cur_code = l.code;
            let n = utf16_len(&piece);
            cur.push_str(&piece);
            used += n;
            first_line = false;
            continue;
        }
        cur.push_str(&piece);
        used += n;
        cur_code = l.code;
        first_line = false;
    }
    if cur_code {
        cur.push_str("</pre>");
    }
    if !cur.trim().is_empty() || bodies.is_empty() {
        bodies.push(cur);
    }
    let n = bodies.len();
    if n == 1 {
        return bodies;
    }
    bodies.into_iter().enumerate().map(|(i, b)| format!("{b}\n({}/{n})", i + 1)).collect()
}

/// One message that must stay one (a card with buttons): as is when it fits, else its visible
/// text cut to fit and escaped again, ending "…".
pub fn fit_one(html: &str, budget: usize) -> String {
    let budget = budget.min(MAX_UNITS - RESERVE);
    if utf16_len(html) <= budget {
        return html.to_string();
    }
    let plain = unescape(&strip_tags(html));
    let mut out = String::new();
    for c in plain.chars() {
        if utf16_len(&escape(&out)) + c.len_utf16() + 8 > budget {
            break;
        }
        out.push(c);
    }
    out.push('…');
    escape(&out)
}

/// Plain text (the fallback when Telegram can't parse the HTML), split the same way.
pub fn plain_parts(text: &str) -> Vec<String> {
    let budget = MAX_UNITS - RESERVE;
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.split('\n') {
        let mut pieces = vec![line.to_string()];
        if utf16_len(line) > budget {
            pieces = Vec::new();
            let mut p = String::new();
            for c in line.chars() {
                if utf16_len(&p) + c.len_utf16() > budget {
                    pieces.push(std::mem::take(&mut p));
                }
                p.push(c);
            }
            pieces.push(p);
        }
        for p in pieces {
            let add = if cur.is_empty() { p.clone() } else { format!("\n{p}") };
            if utf16_len(&cur) + utf16_len(&add) > budget && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
                cur = p;
            } else {
                cur.push_str(&add);
            }
        }
    }
    parts.push(cur);
    let n = parts.len();
    if n == 1 {
        return parts;
    }
    parts.into_iter().enumerate().map(|(i, b)| format!("{b}\n({}/{n})", i + 1)).collect()
}

/// The visible text of our own HTML (tags removed, entities kept).
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tags in `html` open and close in order, and only ours appear.
    fn balanced(html: &str) -> bool {
        let mut stack: Vec<String> = Vec::new();
        let mut rest = html;
        while let Some(i) = rest.find('<') {
            let Some(j) = rest[i..].find('>') else { return false };
            let tag = &rest[i + 1..i + j];
            let name = tag.trim_start_matches('/').split(' ').next().unwrap_or("");
            if !["b", "i", "s", "code", "pre", "a"].contains(&name) {
                return false;
            }
            if tag.starts_with('/') {
                if stack.pop().as_deref() != Some(name) {
                    return false;
                }
            } else {
                stack.push(name.to_string());
            }
            rest = &rest[i + j + 1..];
        }
        stack.is_empty()
    }

    #[test]
    fn markdown_subset() {
        assert_eq!(inline("**Bold** and *it* and `x<y`"), "<b>Bold</b> and <i>it</i> and <code>x&lt;y</code>");
        assert_eq!(inline("snake_case_name and a_b"), "snake_case_name and a_b");
        assert_eq!(inline("[docs](https://example.com/a_(b))"), "<a href=\"https://example.com/a_(b)\">docs</a>");
        assert_eq!(inline("[x](javascript:alert(1))"), "[x](javascript:alert(1))");
        assert_eq!(inline("<script>alert(1)</script> & \"q\""), "&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;q&quot;");
        assert_eq!(inline("unmatched * star and ** two"), "unmatched * star and ** two");
        assert_eq!(inline("~~gone~~"), "<s>gone</s>");
        let p = message_parts(Some("<b>Drafter</b> · #general"), "# Title\n- one\n* two\n> quote\n```\nfn a<T>() {}\n```\nend");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0], "<b>Drafter</b> · #general\n<b>Title</b>\n• one\n• two\n▎ quote\n<pre>fn a&lt;T&gt;() {}</pre>\nend");
        for s in ["_ * [ ] ( ) ~ ` > # + - = | { } . ! \\", "**a *b* c**", "*a **b** c*", "| a | b |\n|---|---|", "1. x\n2. y", "https://x.example/a_b_c"] {
            for part in message_parts(None, s) {
                assert!(balanced(&part), "{s:?} → {part}");
            }
        }
    }

    #[test]
    fn a_long_line_shares_the_header_part_and_breaks_between_words() {
        let text = format!("LONG {}", "word ".repeat(1500));
        let parts = message_parts(Some("<b>Drafter</b> · #general"), &text);
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(parts[0].starts_with("<b>Drafter</b> · #general\nLONG word"), "{}", &parts[0][..60]);
        assert!(parts[0].ends_with("word \n(1/2)"), "the first part ends between words");
        assert!(parts[1].starts_with("word "), "the second part starts with a whole word");
        assert!(parts[1].ends_with("\n(2/2)"));
    }

    #[test]
    fn long_answers_split_under_the_limit_and_lose_nothing() {
        let emoji = "😀".repeat(5000);
        let cyr = "Привет мир ".repeat(800);
        let word = "x".repeat(5000);
        let mut code = String::from("```\n");
        for i in 0..600 {
            code.push_str(&format!("let v{i} = {i};\n"));
        }
        code.push_str("```\nafter");
        let lines_4097 = format!("{}\n{}", "a".repeat(4000), "b".repeat(97));
        for text in [emoji.as_str(), cyr.as_str(), word.as_str(), code.as_str(), lines_4097.as_str(), &"y".repeat(4096), &"z".repeat(8193)] {
            let parts = message_parts(Some("<b>B</b> · #general"), text);
            let n = parts.len();
            for (i, p) in parts.iter().enumerate() {
                assert!(utf16_len(p) <= MAX_UNITS, "part {i} has {} units", utf16_len(p));
                assert!(balanced(p), "part {i} unbalanced");
                if n > 1 {
                    assert!(p.ends_with(&format!("({}/{n})", i + 1)));
                }
            }
            // Joined again (suffixes, header and line breaks aside), nothing is lost or doubled
            let joined: String = parts
                .iter()
                .map(|p| {
                    let p = if n > 1 { p.rsplit_once('\n').map(|x| x.0).unwrap_or(p) } else { p.as_str() };
                    unescape(&strip_tags(p))
                })
                .collect::<Vec<_>>()
                .join("");
            let want: String = text.replace("```\n", "").replace("```", "").chars().filter(|c| *c != '\n').collect();
            let got: String = joined.trim_start_matches("B · #general").chars().filter(|c| *c != '\n').collect();
            assert_eq!(got, want);
        }
        assert_eq!(message_parts(None, "short").len(), 1);
    }

    #[test]
    fn plain_fallback_splits_too() {
        let parts = plain_parts(&"é😀".repeat(3000));
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| utf16_len(p) <= MAX_UNITS));
    }
}
