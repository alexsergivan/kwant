//! Readable text from an HTML mail part, ported from ClankerInbox's `htmlToText` (`src/mime.ts`,
//! MIT). HTML-only mails still need text for answers and codes; links keep their target. One
//! pass from the left, never a backtracking pattern over the whole mail: anyone can send HTML.

use std::collections::HashSet;

const HIDDEN: &[&str] = &["script", "style", "head"];
const BLOCK_END: &[&str] = &[
    "p",
    "div",
    "tr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "blockquote",
];

/// The tag name: `^/?([a-z][a-z0-9]*)`, lowercase.
fn tag_name(tag: &str) -> String {
    let t = tag.strip_prefix('/').unwrap_or(tag);
    let mut it = t.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() => {
            let mut s = String::from(c.to_ascii_lowercase());
            s.extend(
                it.take_while(|c| c.is_ascii_alphanumeric())
                    .map(|c| c.to_ascii_lowercase()),
            );
            s
        }
        _ => String::new(),
    }
}

/// `\shref\s*=\s*["']([^"']+)["']` in `' ' + tag[1..]`.
fn href_of(tag: &str) -> Option<String> {
    let rest: String = format!(" {}", tag.get(1..).unwrap_or(""));
    let lower = rest.to_ascii_lowercase();
    let mut from = 0;
    while let Some(k) = lower[from..].find("href") {
        let at = from + k;
        from = at + 4;
        if at == 0 || !lower[..at].ends_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let tail = &rest[at + 4..];
        let tail = tail.trim_start();
        let Some(tail) = tail.strip_prefix('=') else {
            continue;
        };
        let tail = tail.trim_start();
        if !tail.starts_with(['"', '\'']) {
            continue;
        }
        let body = &tail[1..];
        match body.find(['"', '\'']) {
            Some(e) if e > 0 => return Some(body[..e].to_string()),
            _ => continue,
        }
    }
    None
}

/// The index just past `</name ...>` at or after `from`, or `None`.
fn closing_tag(html: &str, name: &str, from: usize) -> Option<usize> {
    let mut at = html[from..].find("</").map(|k| from + k);
    while let Some(a) = at {
        let after = a + 2 + name.len();
        if html
            .get(a + 2..after)
            .is_some_and(|s| s.eq_ignore_ascii_case(name))
            && !html[after..].starts_with(|c: char| c.is_ascii_alphanumeric())
        {
            return html[after..].find('>').map(|g| after + g + 1);
        }
        at = html[a + 2..].find("</").map(|k| a + 2 + k);
    }
    None
}

/// Whether the text so far ends in the middle of a line (after text that is not white space).
fn in_line(out: &[String]) -> bool {
    for piece in out.iter().rev() {
        if piece.trim().is_empty() {
            if piece.contains('\n') {
                return false;
            }
            continue;
        }
        // !/\n\s*$/: the white space it ends with holds no line break
        return !piece[piece.trim_end().len()..].contains('\n');
    }
    false
}

fn code_point(n: u32, written: &str) -> String {
    match char::from_u32(n).filter(|_| n > 0) {
        Some(c) => c.to_string(),
        None => written.to_string(),
    }
}

/// `&#(\d{1,7});` and `&#x([0-9a-f]{1,6});` (any case), each decoded or left as written.
fn numeric_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(k) = s[i..].find("&#") {
        let at = i + k;
        out.push_str(&s[i..at]);
        let rest = &s[at + 2..];
        let (hex, digits_start) = match rest.chars().next() {
            Some('x') | Some('X') => (true, 1),
            _ => (false, 0),
        };
        let body = &rest[digits_start..];
        let len = body
            .chars()
            .take_while(|c| {
                if hex {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            })
            .count();
        let max = if hex { 6 } else { 7 };
        if len >= 1 && len <= max && body[len..].starts_with(';') {
            let written = &s[at..at + 2 + digits_start + len + 1];
            let n = u32::from_str_radix(&body[..len], if hex { 16 } else { 10 }).unwrap_or(0);
            out.push_str(&code_point(n, written));
            i = at + written.len();
        } else {
            out.push_str("&#");
            i = at + 2;
        }
    }
    out.push_str(&s[i..]);
    out
}

pub fn html_to_text(html: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    // Closing tags searched for once and not found: a later search wouldn't find them either
    let mut missing: HashSet<String> = HashSet::new();
    let mut href: Option<String> = None;
    let mut i = 0;
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|k| i + k) else {
            out.push(html[i..].to_string());
            break;
        };
        out.push(html[i..lt].to_string());
        // No `>` anywhere after this: the rest is text
        let Some(gt) = html[lt + 1..].find('>').map(|k| lt + 1 + k) else {
            out.push(html[lt..].to_string());
            break;
        };
        if gt == lt + 1 {
            out.push("<>".into());
            i = gt + 1;
            continue;
        }
        let tag = &html[lt + 1..gt];
        let closing = tag.starts_with('/');
        let name = tag_name(tag);
        i = gt + 1;
        if !closing && HIDDEN.contains(&name.as_str()) && !missing.contains(&name) {
            match closing_tag(html, &name, i) {
                None => {
                    missing.insert(name.clone());
                }
                Some(end) => {
                    out.push(" ".into());
                    i = end;
                    continue;
                }
            }
        }
        let block = BLOCK_END.contains(&name.as_str());
        if name == "a" && !closing && href.is_none() {
            href = href_of(tag);
        } else if name == "a" && closing && href.is_some() {
            out.push(format!(" {} ", href.take().unwrap_or_default()));
        } else if name == "br" || (closing && block) {
            out.push("\n".into());
        } else if !closing && name == "blockquote" {
            out.push("\n> ".into());
        } else if !closing && block && in_line(&out) {
            // A block that starts inside a line of text (`line 1<div>line 2</div>`) starts a new line
            out.push("\n".into());
        }
    }
    let joined = out.concat();
    let s = joined
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let s = numeric_entities(&s).replace("&amp;", "&");
    let s = s
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let mut squeezed = String::with_capacity(s.len());
    let mut run = 0;
    for c in s.chars() {
        if c == '\n' {
            run += 1;
            if run > 2 {
                continue;
            }
        } else {
            run = 0;
        }
        squeezed.push(c);
    }
    squeezed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_scripts_and_styles_keeps_link_targets_and_line_breaks() {
        let html = "<HEAD><title>t</title></head><style>p{}</STYLE><header>Hi</header><p>Code <b>12</b></p>\
            <script>alert(1)</script><a class=\"x\" HREF=\"https://a.example/v?t=1\">open</a><br/>end<blockquote>q</blockquote>< not a tag";
        assert_eq!(
            html_to_text(html),
            "Hi\nCode 12\n open https://a.example/v?t=1\nend\n> q\n< not a tag"
        );
    }

    #[test]
    fn numeric_entities_outside_unicode_stay_as_written_hex_entities_decode() {
        assert_eq!(
            html_to_text("<p>&#99999999; &#x41; &#65; &#x110000;</p>"),
            "&#99999999; A A &#x110000;"
        );
        assert_eq!(
            html_to_text("a &amp;lt; b &lt;c&gt; &quot;d&quot; &#39;e&#39;&nbsp;f"),
            "a &lt; b <c> \"d\" 'e' f"
        );
    }

    #[test]
    fn a_link_and_entities_in_a_sentence() {
        let t = html_to_text("<p>Click <a href=\"https://app.example.com/verify?t=abc\">here</a> to sign in &amp; go.</p>");
        assert!(
            t.contains("here https://app.example.com/verify?t=abc"),
            "{t}"
        );
        assert!(t.contains("sign in & go."), "{t}");
    }

    #[test]
    fn a_block_inside_a_line_starts_a_new_line() {
        assert_eq!(
            html_to_text(
                "<div dir=\"ltr\">Not yet.<div>The tests are red.</div><br clear=\"all\"><div>"
            ),
            "Not yet.\nThe tests are red."
        );
        assert_eq!(html_to_text("<>x"), "<>x");
    }

    #[test]
    fn hostile_input_is_linear() {
        let t0 = std::time::Instant::now();
        let _ = html_to_text(&"<script>".repeat(200_000));
        let _ = html_to_text(&"<".repeat(1_000_000));
        let _ = html_to_text(&"&#".repeat(500_000));
        assert!(t0.elapsed().as_secs() < 5, "{:?}", t0.elapsed());
    }
}
